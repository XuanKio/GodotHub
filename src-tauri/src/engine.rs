use crate::{
    archive,
    catalog::{Asset, Edition},
    jobs::unix_millis,
    storage::{canonical_path, EngineRecord, Store},
    windows_host,
};
use serde::Serialize;
use std::{
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
    time::Duration,
};
use uuid::Uuid;

pub(crate) const OWNER_MARKER: &str = ".godot-hub-install";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledEngine {
    #[serde(flatten)]
    record: EngineSummary,
    managed: bool,
    available: bool,
    is_default: bool,
    templates_installed: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EngineSummary {
    id: String,
    tag: String,
    edition: Edition,
    arch: String,
    executable: PathBuf,
    installed_at: u64,
}

pub fn same_build(record: &EngineRecord, tag: &str, edition: Edition, arch: &str) -> bool {
    record.tag == tag && record.edition == edition && record.arch == arch
}

pub fn list(store: &Store, godot_data: &Path) -> Vec<InstalledEngine> {
    store.read(|data| {
        data.engines
            .iter()
            .map(|record| view(record, data.default_engine_id.as_deref(), godot_data))
            .collect()
    })
}

fn view(record: &EngineRecord, default: Option<&str>, godot_data: &Path) -> InstalledEngine {
    InstalledEngine {
        record: EngineSummary {
            id: record.id.clone(),
            tag: record.tag.clone(),
            edition: record.edition,
            arch: record.arch.clone(),
            executable: record.executable.clone(),
            installed_at: record.installed_at,
        },
        managed: record.managed_directory.is_some(),
        available: record.executable.is_file(),
        is_default: default == Some(record.id.as_str()),
        templates_installed: templates_installed(record, godot_data),
    }
}

pub fn get(store: &Store, id: &str) -> Result<EngineRecord, String> {
    store
        .read(|state| state.engines.iter().find(|record| record.id == id).cloned())
        .ok_or_else(|| "This installation is no longer registered.".into())
}

pub fn set_default(store: &Store, id: &str) -> Result<(), String> {
    store.update(|state| {
        if !state
            .engines
            .iter()
            .any(|record| record.id == id && record.executable.is_file())
        {
            return Err(
                "The engine executable is missing. Reinstall or reimport the engine.".into(),
            );
        }
        state.default_engine_id = Some(id.to_owned());
        Ok(())
    })
}

pub fn engine_directory_name(tag: &str, edition: Edition, arch: &str) -> Result<String, String> {
    if !valid_tag(tag) || !matches!(arch, "x64" | "x86" | "arm64") {
        return Err("The Godot build has an invalid version or architecture.".into());
    }
    Ok(format!("Godot-{tag}-{}-{arch}", edition.as_str()))
}

pub fn valid_tag(tag: &str) -> bool {
    !tag.is_empty()
        && tag.len() <= 64
        && tag.starts_with(|ch: char| ch.is_ascii_digit())
        && tag
            .bytes()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == b'.' || ch == b'-')
}

pub fn unpack_engine(archive: &Path, staging: &Path, arch: &str) -> Result<PathBuf, String> {
    archive::extract(archive, staging)?;
    let executable = find_executable(staging, 2, arch)?;
    executable
        .ok_or_else(|| {
            format!("This archive does not contain a Godot editor executable for {arch}.")
        })?
        .strip_prefix(staging)
        .map(Path::to_path_buf)
        .map_err(|e| e.to_string())
}

fn find_executable(folder: &Path, depth: u8, arch: &str) -> Result<Option<PathBuf>, String> {
    let entries =
        fs::read_dir(folder).map_err(|e| format!("Cannot inspect extracted editor: {e}"))?;
    let mut directories = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| format!("Cannot inspect extracted editor: {e}"))?;
        let path = entry.path();
        if path.is_dir() {
            directories.push(path);
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
        if name.starts_with("godot")
            && name.ends_with(".exe")
            && !name.contains("console")
            && windows_host::executable_arch(&path).is_ok_and(|value| value == arch)
        {
            return Ok(Some(path));
        }
    }
    if depth > 0 {
        for directory in directories {
            if let Some(executable) = find_executable(&directory, depth - 1, arch)? {
                return Ok(Some(executable));
            }
        }
    }
    Ok(None)
}

pub fn commit_install(
    store: &Store,
    mut record: EngineRecord,
    staging: &Path,
    final_path: &Path,
) -> Result<(), String> {
    let relative = record.executable.clone();
    if major(&record) == 2 {
        // Godot 2 uses a single unversioned template folder. Isolate each managed editor.
        let editor = staging.join(relative.parent().unwrap_or(Path::new("")));
        fs::write(editor.join("_sc_"), "")
            .map_err(|e| format!("Cannot enable self-contained mode for Godot 2: {e}"))?;
    }
    record.executable = final_path.join(relative);
    record.managed_directory = Some(final_path.to_owned());
    fs::write(staging.join(OWNER_MARKER), &record.id)
        .map_err(|e| format!("Cannot mark the managed engine directory: {e}"))?;
    if final_path.exists() {
        return Err(format!("An engine directory already exists at {}. Move it or choose a different install location.", final_path.display()));
    }
    fs::rename(staging, final_path)
        .map_err(|e| format!("Cannot move the engine into {}: {e}", final_path.display()))?;
    if let Err(error) = register(store, record) {
        fs::rename(final_path, staging).map_err(|rollback| {
            format!(
                "{error}. Could not roll back {}: {rollback}",
                final_path.display()
            )
        })?;
        return Err(error);
    }
    Ok(())
}

fn register(store: &Store, record: EngineRecord) -> Result<(), String> {
    store.update(|state| {
        if state
            .engines
            .iter()
            .any(|existing| paths_match(&existing.executable, &record.executable))
        {
            return Err("This executable is already registered.".into());
        }
        if record.managed_directory.is_some()
            && state.engines.iter().any(|existing| {
                existing.managed_directory.is_some()
                    && same_build(existing, &record.tag, record.edition, &record.arch)
            })
        {
            return Err("This build is already installed by Godot Hub.".into());
        }
        if state.default_engine_id.is_none() {
            state.default_engine_id = Some(record.id.clone());
        }
        state.engines.insert(0, record);
        Ok(())
    })
}

fn paths_match(first: &Path, second: &Path) -> bool {
    first == second
        || first
            .to_string_lossy()
            .eq_ignore_ascii_case(&second.to_string_lossy())
}

pub async fn import(
    store: &Store,
    godot_data: &Path,
    executable: PathBuf,
) -> Result<InstalledEngine, String> {
    if !executable.is_absolute()
        || !executable
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
    {
        return Err("Choose an extracted Windows Godot .exe file.".into());
    }
    let executable =
        canonical_path(&executable).map_err(|e| format!("Cannot locate that executable: {e}"))?;
    if store.read(|state| {
        state
            .engines
            .iter()
            .any(|record| paths_match(&record.executable, &executable))
    }) {
        return Err("This executable is already registered.".into());
    }
    let arch = windows_host::executable_arch(&executable)?;
    if !windows_host::compatible(arch) {
        return Err(format!(
            "This {arch} build is incompatible with this Windows installation."
        ));
    }
    let mut command = tokio::process::Command::new(&executable);
    command
        .arg("--help")
        .current_dir(executable.parent().expect("executable has a parent"))
        .kill_on_drop(true)
        .creation_flags(0x08000000);
    let output = tokio::time::timeout(Duration::from_secs(10), command.output())
        .await
        .map_err(|_| "Godot did not report its version within 10 seconds.".to_owned())?
        .map_err(|e| format!("Cannot run the selected editor to read its version: {e}"))?;
    let help = String::from_utf8_lossy(&output.stdout);
    let (tag, edition) = parse_help_version(&help, output.status.success())
        .ok_or_else(|| "The executable did not report a recognized Godot version. Select the Godot editor executable.".to_owned())?;
    let record = EngineRecord {
        id: Uuid::new_v4().to_string(),
        tag,
        edition,
        arch: arch.into(),
        executable,
        managed_directory: None,
        installed_at: unix_millis(),
    };
    register(store, record.clone())?;
    Ok(store.read(|state| view(&record, state.default_engine_id.as_deref(), godot_data)))
}

fn parse_help_version(help: &str, success: bool) -> Option<(String, Edition)> {
    let (tag, edition) = help.lines().find_map(|line| {
        // Recent Godot GUI builds color the banner even with stdout redirected.
        let mut sections = line.split("\x1b[");
        let mut banner = sections.next()?.to_owned();
        for section in sections {
            let (_, text) = section.split_once('m')?;
            banner.push_str(text);
        }
        parse_version(
            banner
                .strip_prefix("Godot Engine v")?
                .split_whitespace()
                .next()?,
        )
    })?;
    // Godot 2 prints its help banner and exits with a nonzero status.
    (success || legacy_cli(&tag)).then_some((tag, edition))
}

pub fn legacy_cli(tag: &str) -> bool {
    tag.starts_with("1.") || tag.starts_with("2.")
}

fn parse_version(version: &str) -> Option<(String, Edition)> {
    let mut parts = version.trim().split('.').peekable();
    let mut numbers = Vec::new();
    while parts
        .peek()
        .is_some_and(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
    {
        numbers.push(parts.next()?);
    }
    if !(2..=4).contains(&numbers.len()) {
        return None;
    }
    let status = parts.next()?;
    let valid_status = ["stable", "beta", "rc", "dev", "alpha"]
        .iter()
        .any(|&prefix| {
            status
                .strip_prefix(prefix)
                .is_some_and(|number| number.bytes().all(|b| b.is_ascii_digit()))
        });
    if !valid_status {
        return None;
    }
    let tag = format!("{}-{status}", numbers.join("."));
    let edition = if parts.any(|token| matches!(token, "mono" | "dotnet")) {
        Edition::Dotnet
    } else {
        Edition::Standard
    };
    valid_tag(&tag).then_some((tag, edition))
}

pub fn remove(store: &Store, record: &EngineRecord) -> Result<(), String> {
    let mut moved = None;
    if let Some(directory) = &record.managed_directory {
        if directory.exists() {
            let marker = directory.join(OWNER_MARKER);
            let canonical = canonical_path(directory)
                .map_err(|e| format!("Cannot resolve the managed installation: {e}"))?;
            if !paths_match(&canonical, directory)
                || fs::read_to_string(marker).as_deref().ok() != Some(record.id.as_str())
            {
                return Err(
                    "The installed directory no longer matches Godot Hub's ownership record."
                        .into(),
                );
            }
            let parent = directory
                .parent()
                .ok_or("The managed directory has no parent.")?;
            let trash = tempfile::Builder::new()
                .prefix(".godot-hub-removing-")
                .tempdir_in(parent)
                .map_err(|e| format!("Cannot prepare the engine removal: {e}"))?;
            let temporary = trash.path().join("engine");
            fs::rename(directory, &temporary)
                .map_err(|e| format!("Cannot remove the engine. Close Godot and try again. {e}"))?;
            moved = Some((trash, temporary, directory));
        }
    }
    let update = store.update(|state| {
        state.engines.retain(|engine| engine.id != record.id);
        if state.default_engine_id.as_deref() == Some(record.id.as_str()) {
            state.default_engine_id = state
                .engines
                .iter()
                .find(|engine| engine.executable.is_file())
                .map(|engine| engine.id.clone());
        }
        Ok(())
    });
    if let Some((trash, temporary, original)) = moved {
        if let Err(error) = update {
            if let Err(rollback) = fs::rename(&temporary, original) {
                let recovery = trash.keep();
                return Err(format!(
                    "{error}. The engine can be recovered from {}: {rollback}",
                    recovery.display()
                ));
            }
            return Err(error);
        }
        let path = trash.path().to_owned();
        trash.close().map_err(|e| {
            format!(
                "Engine registration removed. Some files in {} could not be deleted: {e}",
                path.display()
            )
        })?;
    }
    update
}

fn template_root(record: &EngineRecord, godot_data: &Path) -> PathBuf {
    let portable = record
        .executable
        .parent()
        .filter(|folder| folder.join("._sc_").is_file() || folder.join("_sc_").is_file());
    let data = portable
        .map(|folder| folder.join("editor_data"))
        .unwrap_or_else(|| godot_data.to_owned());
    data.join(if record.tag.starts_with("4.") || major(record) > 4 {
        "export_templates"
    } else {
        "templates"
    })
}

fn major(record: &EngineRecord) -> u8 {
    record
        .tag
        .split('.')
        .next()
        .and_then(|value| value.parse().ok())
        .unwrap_or(0)
}

fn template_version(record: &EngineRecord, dotnet: bool) -> String {
    if major(record) < 3 {
        return record.tag.clone();
    }
    let version = record.tag.replacen('-', ".", 1);
    match record.edition {
        Edition::Standard => version,
        Edition::Dotnet if dotnet => format!("{version}.dotnet"),
        Edition::Dotnet => format!("{version}.mono"),
    }
}

fn uses_dotnet_name(record: &EngineRecord) -> bool {
    record.executable.file_name().is_some_and(|name| {
        name.to_string_lossy()
            .to_ascii_lowercase()
            .contains("dotnet")
    })
}

pub fn templates_installed(record: &EngineRecord, godot_data: &Path) -> bool {
    let version = template_version(record, uses_dotnet_name(record));
    template_files_present(
        &templates_destination(record, godot_data, &version),
        &version,
        major(record),
    )
}

fn template_files_present(path: &Path, version: &str, major: u8) -> bool {
    if major >= 2 && read_version(path).as_deref() != Some(version) {
        return false;
    }
    fs::read_dir(path).is_ok_and(|entries| {
        entries.filter_map(Result::ok).any(|entry| {
            entry.file_type().is_ok_and(|kind| kind.is_file()) && {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                name.starts_with("windows_") && name.ends_with(".exe")
            }
        })
    })
}

fn read_version(path: &Path) -> Option<String> {
    let mut version = String::new();
    fs::File::open(path.join("version.txt"))
        .ok()?
        .take(1024)
        .read_to_string(&mut version)
        .ok()?;
    Some(version.trim().to_owned())
}

pub fn templates_staging_root(record: &EngineRecord, godot_data: &Path) -> Result<PathBuf, String> {
    let root = template_root(record, godot_data);
    let root = if major(record) < 3 {
        root.parent().unwrap_or(&root).to_owned()
    } else {
        root
    };
    fs::create_dir_all(&root)
        .map_err(|e| format!("Cannot create the Godot export templates folder: {e}"))?;
    Ok(root)
}

pub fn templates_destination(record: &EngineRecord, godot_data: &Path, version: &str) -> PathBuf {
    let root = template_root(record, godot_data);
    if major(record) < 3 {
        root
    } else {
        root.join(version)
    }
}

pub fn unpack_templates(
    archive: &Path,
    staging: &Path,
    record: &EngineRecord,
    asset: &Asset,
) -> Result<(PathBuf, String), String> {
    archive::extract(archive, staging)?;
    let payload = staging.join("templates");
    let payload = if payload.is_dir() {
        payload
    } else {
        staging.to_owned()
    };
    let version = template_version(record, asset.name.to_ascii_lowercase().contains("dotnet"));
    if !template_files_present(&payload, &version, major(record)) {
        return Err(format!("The export templates archive is missing Windows templates or does not match version {version}."));
    }
    Ok((payload, version))
}

pub fn commit_templates(
    payload: &Path,
    destination: &Path,
    version: &str,
    record: &EngineRecord,
) -> Result<(), String> {
    if template_files_present(destination, version, major(record)) {
        return Ok(());
    }
    let replaced_empty = match fs::symlink_metadata(destination) {
        Ok(metadata) => {
            use std::os::windows::fs::MetadataExt;
            const REPARSE_POINT: u32 = 0x0400;
            if !metadata.is_dir()
                || metadata.file_attributes() & REPARSE_POINT != 0
                || fs::remove_dir(destination).is_err()
            {
                return Err(format!(
                    "Another export template folder already exists at {}. Move it before retrying.",
                    destination.display()
                ));
            }
            true
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => false,
        Err(error) => {
            return Err(format!(
                "Cannot check the export templates directory: {error}"
            ))
        }
    };
    fs::rename(payload, destination).map_err(|e| {
        if replaced_empty {
            let _ = fs::create_dir(destination);
        }
        format!(
            "Cannot move export templates into {}: {e}",
            destination.display()
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates_can_replace_the_empty_directory_created_by_godot() {
        let workspace = tempfile::tempdir().unwrap();
        let payload = workspace.path().join("payload");
        let destination = workspace.path().join("templates");
        fs::create_dir(&payload).unwrap();
        fs::create_dir(&destination).unwrap();
        fs::write(payload.join("version.txt"), "2.1.6-stable\n").unwrap();
        fs::write(payload.join("windows_64_debug.exe"), "fixture").unwrap();
        let record = EngineRecord {
            id: "test".into(),
            tag: "2.1.6-stable".into(),
            edition: Edition::Standard,
            arch: "x64".into(),
            executable: workspace.path().join("godot.exe"),
            managed_directory: None,
            installed_at: 0,
        };
        let user_file = destination.join("user-file");
        fs::write(&user_file, "preserve").unwrap();
        assert!(commit_templates(&payload, &destination, &record.tag, &record).is_err());
        assert_eq!(fs::read_to_string(&user_file).unwrap(), "preserve");
        fs::remove_file(user_file).unwrap();
        commit_templates(&payload, &destination, &record.tag, &record).unwrap();
        assert!(template_files_present(&destination, &record.tag, 2));
    }

    #[test]
    fn parses_official_engine_version_output() {
        for (output, tag, edition) in [
            (
                "4.7.2.stable.mono.official.ed1daf0bf",
                "4.7.2-stable",
                Edition::Dotnet,
            ),
            ("4.8.beta5.official.764240b", "4.8-beta5", Edition::Standard),
            ("3.6.3.stable.official", "3.6.3-stable", Edition::Standard),
            ("4.9.dev1.dotnet.official.123", "4.9-dev1", Edition::Dotnet),
        ] {
            let (parsed_tag, parsed_edition) = parse_version(output).unwrap();
            assert_eq!(parsed_tag, tag);
            assert!(parsed_edition == edition);
        }
        assert!(parse_version("Microsoft Windows 11").is_none());
    }

    #[test]
    fn reads_legacy_and_modern_help_banners() {
        let old = "Godot Engine v2.1.6.stable.official - https://godotengine.org\nUsage: godot ...";
        let current = "\x1b[38;5;39mGodot Engine\x1b[0m v4.7.2.stable.mono.official.ed1daf0bf - \x1b[4mhttps://godotengine.org\x1b[0m";
        assert_eq!(
            parse_help_version(old, false),
            Some(("2.1.6-stable".into(), Edition::Standard))
        );
        assert_eq!(
            parse_help_version(current, true),
            Some(("4.7.2-stable".into(), Edition::Dotnet))
        );
        assert!(parse_help_version(current, false).is_none());
    }
}
