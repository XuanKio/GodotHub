use crate::{archive, storage::canonical_path};
use reqwest::{redirect, Client, Response, Url};
use serde::{de::Error as _, Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

const ASSET_API: &str = "https://godotengine.org/asset-library/api/asset";
const ADDONS: &str = "addons";
const PLUGIN_CONFIG: &str = "plugin.cfg";
const OWNER: &str = ".godot-hub-plugin.json";
const BACKUPS: &str = ".godot-hub-plugin-backups";
const MAX_ARCHIVE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_EXTRACTED_BYTES: u64 = 512 * 1024 * 1024;
const MAX_ARCHIVE_ENTRIES: usize = 10_000;
const MAX_MANIFEST_BYTES: u64 = 256 * 1024;
const MAX_API_BYTES: usize = 2 * 1024 * 1024;
const RESULTS_PER_PAGE: u32 = 12;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectPlugin {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub author: String,
    pub directory: PathBuf,
    pub managed: bool,
}

#[derive(Serialize, Deserialize)]
pub struct LibraryPlugin {
    #[serde(rename(deserialize = "asset_id"), deserialize_with = "number")]
    pub id: u64,
    pub title: String,
    pub author: String,
    #[serde(rename(deserialize = "version_string"))]
    pub version: String,
    pub category: String,
    #[serde(rename(deserialize = "cost"))]
    pub license: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetSearch {
    pub assets: Vec<LibraryPlugin>,
    pub page: u32,
    pub page_count: u32,
}

#[derive(Deserialize)]
struct ApiSearch {
    result: Vec<LibraryPlugin>,
    page: u32,
    page_length: u32,
    total_items: u32,
}

#[derive(Deserialize)]
struct ApiAsset {
    #[serde(rename = "type")]
    kind: String,
    download_url: String,
    download_hash: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct Owner {
    schema: u8,
    project: PathBuf,
    addon_id: String,
}

pub fn list(project: &Path) -> Result<Vec<ProjectPlugin>, String> {
    let project = project_root(project)?;
    let addons = project.join(ADDONS);
    if !directory_exists(&addons)? {
        return Ok(Vec::new());
    }
    let mut plugins = Vec::new();
    for path in immediate_directories(&addons)? {
        let Some(id) = addon_id(&path) else { continue };
        if let Ok(Some(mut plugin)) = plugin_at(&path, id) {
            plugin.managed = managed_by_hub(&project, &path, id);
            plugins.push(plugin);
        }
    }
    plugins.sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok(plugins)
}

pub fn install_zip(project: &Path, zip: &Path) -> Result<Vec<ProjectPlugin>, String> {
    let project = project_root(project)?;
    if !zip.is_absolute()
        || !zip
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("zip"))
    {
        return Err("Choose a local ZIP archive containing a Godot plugin.".into());
    }
    let file = open_unlinked_file(zip)?
        .ok_or_else(|| format!("ZIP archive not found at {}.", zip.display()))?;
    if file
        .metadata()
        .map_err(|error| format!("Cannot check the ZIP archive: {error}"))?
        .len()
        > MAX_ARCHIVE_BYTES
    {
        return Err("The plugin ZIP exceeds 128 MiB.".into());
    }

    let addons = project.join(ADDONS);
    directory_exists(&addons)?;
    let staging = tempfile::Builder::new()
        .prefix(".godot-hub-plugin-stage-")
        .tempdir_in(&project)
        .map_err(|error| format!("Cannot prepare plugin staging: {error}"))?;
    create_ignore(staging.path())?;
    let payload = staging.path().join("payload");
    archive::extract_file(file, &payload, MAX_ARCHIVE_ENTRIES, MAX_EXTRACTED_BYTES)?;

    let candidates = package_addons(&payload, zip)?;
    let mut checked = Vec::new();
    let mut ids = HashSet::new();
    for (source, id) in candidates {
        let Some(mut plugin) = plugin_at(&source, &id)? else {
            continue;
        };
        if !ids.insert(id.to_lowercase()) {
            return Err(format!(
                "The archive contains duplicate addon folders named {id}."
            ));
        }
        let destination = addons.join(&id);
        require_vacant(&destination)?;
        write_owner(&project, &source, &id)?;
        plugin.directory = destination;
        plugin.managed = true;
        checked.push((source, plugin));
    }
    if checked.is_empty() {
        return Err(
            "No Godot plugin found. The ZIP must contain an addon folder with plugin.cfg.".into(),
        );
    }

    let created_addons = ensure_directory(&addons)?;
    let mut committed = 0;
    for (source, plugin) in &checked {
        let result = directory_exists(&addons).and_then(|exists| {
            if !exists {
                return Err("The project's addons folder disappeared.".into());
            }
            require_vacant(&plugin.directory)?;
            fs::rename(source, &plugin.directory)
                .map_err(|error| format!("Cannot install addon {}: {error}", plugin.id))
        });
        if let Err(error) = result {
            let remaining = checked[..committed]
                .iter()
                .rev()
                .filter_map(|(source, plugin)| {
                    let restored = directory_exists(&addons).and_then(|exists| {
                        if !exists {
                            return Err("The project's addons folder disappeared.".into());
                        }
                        fs::rename(&plugin.directory, source).map_err(|error| error.to_string())
                    });
                    restored.err().map(|_| plugin.id.as_str())
                })
                .collect::<Vec<_>>();
            if created_addons {
                let _ = fs::remove_dir(&addons);
            }
            return if remaining.is_empty() {
                Err(error)
            } else {
                Err(format!(
                    "{error} The following addons remain installed: {}.",
                    remaining.join(", ")
                ))
            };
        }
        committed += 1;
    }
    Ok(checked.into_iter().map(|(_, plugin)| plugin).collect())
}

pub fn remove(project: &Path, id: &str) -> Result<PathBuf, String> {
    if !valid_id(id) {
        return Err("Invalid Godot addon folder name.".into());
    }
    let project = project_root(project)?;
    let addons = project.join(ADDONS);
    let source = addons.join(id);
    if !directory_exists(&addons)? || !directory_exists(&source)? {
        return Err("This addon folder is missing.".into());
    }
    if !managed_by_hub(&project, &source, id) {
        return Err("This addon was not installed by Godot Hub.".into());
    }

    let backups = project.join(BACKUPS);
    ensure_directory(&backups)?;
    create_ignore(&backups)?;
    let destination = backups.join(Uuid::new_v4().to_string());
    require_vacant(&destination)?;
    if !directory_exists(&addons)?
        || !directory_exists(&source)?
        || !directory_exists(&backups)?
        || !managed_by_hub(&project, &source, id)
    {
        return Err("The addon or its backup location changed. Refresh and try again.".into());
    }
    fs::rename(&source, &destination).map_err(|error| {
        format!("Cannot move addon {id} to a backup. Close any program using its files: {error}")
    })?;
    Ok(destination)
}

pub async fn search(
    query: &str,
    page: u32,
    godot_version: Option<&str>,
) -> Result<AssetSearch, String> {
    let client = api_client()?;
    let mut request = client
        .get(ASSET_API)
        .query(&[
            ("type", "addon"),
            ("filter", query.trim()),
            ("sort", "updated"),
            ("reverse", "1"),
        ])
        .query(&[("page", page), ("max_results", RESULTS_PER_PAGE)]);
    if let Some(version) = godot_version {
        request = request.query(&[("godot_version", version)]);
    }
    let raw: ApiSearch = api_json(request).await?;
    let length = raw.page_length.max(1);
    Ok(AssetSearch {
        assets: raw.result,
        page: raw.page,
        page_count: raw.total_items.div_ceil(length),
    })
}

pub async fn install_library(project: &Path, id: u64) -> Result<Vec<ProjectPlugin>, String> {
    if id == 0 {
        return Err("Invalid Godot Asset Library ID.".into());
    }
    let asset: ApiAsset = api_json(api_client()?.get(format!("{ASSET_API}/{id}"))).await?;
    if asset.kind != "addon" {
        return Err("Choose an addon from the Godot Asset Library.".into());
    }
    let url = Url::parse(&asset.download_url).ok().filter(supported_download_url)
        .ok_or("This asset has no supported HTTPS download. Download its ZIP manually and use Install from ZIP.")?;
    let checksum = asset
        .download_hash
        .as_deref()
        .filter(|hash| !hash.is_empty());
    if checksum
        .is_some_and(|hash| hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()))
    {
        return Err("The Asset Library supplied an invalid SHA-256 checksum.".into());
    }
    let client = Client::builder()
        .user_agent(concat!("Godot-Hub/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(15))
        .read_timeout(Duration::from_secs(45))
        .timeout(Duration::from_secs(600))
        .redirect(redirect::Policy::custom(|attempt| {
            if attempt.previous().len() < 5 && supported_download_url(attempt.url()) {
                attempt.follow()
            } else {
                attempt.error("Unexpected redirect for an Asset Library download")
            }
        }))
        .build()
        .map_err(|error| format!("Cannot start the addon download: {error}"))?;
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|error| format!("Cannot download the addon: {}", error.without_url()))?;
    let archive = download_archive(response, checksum).await?;
    let project = project.to_owned();
    tokio::task::spawn_blocking(move || install_zip(&project, archive.path()))
        .await
        .map_err(|error| format!("Cannot install the downloaded addon: {error}"))?
}

fn api_client() -> Result<Client, String> {
    Client::builder()
        .user_agent(concat!("Godot-Hub/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(25))
        .redirect(redirect::Policy::none())
        .build()
        .map_err(|error| format!("Cannot start Asset Library requests: {error}"))
}

async fn api_json<T: serde::de::DeserializeOwned>(
    request: reqwest::RequestBuilder,
) -> Result<T, String> {
    let mut response = request.send().await.map_err(|error| {
        format!(
            "Cannot reach the Godot Asset Library: {}",
            error.without_url()
        )
    })?;
    if !response.status().is_success() {
        return Err(format!(
            "Godot Asset Library returned HTTP {}.",
            response.status().as_u16()
        ));
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|error| {
        format!(
            "Cannot read the Asset Library response: {}",
            error.without_url()
        )
    })? {
        if chunk.len() > MAX_API_BYTES - body.len() {
            return Err("The Asset Library response is too large.".into());
        }
        body.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&body)
        .map_err(|error| format!("Cannot read the Asset Library response: {error}"))
}

fn number<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Number {
        Value(u64),
        Text(String),
    }
    match Number::deserialize(deserializer)? {
        Number::Value(value) => Ok(value),
        Number::Text(value) => value.parse().map_err(D::Error::custom),
    }
}

fn supported_download_url(url: &Url) -> bool {
    url.scheme() == "https"
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
        && matches!(
            url.host_str(),
            Some(
                "github.com"
                    | "codeload.github.com"
                    | "release-assets.githubusercontent.com"
                    | "objects.githubusercontent.com"
                    | "gitlab.com"
                    | "bitbucket.org"
                    | "api.bitbucket.org"
                    | "bbuseruploads.s3.amazonaws.com"
                    | "codeberg.org"
            )
        )
}

async fn download_archive(
    mut response: Response,
    checksum: Option<&str>,
) -> Result<tempfile::NamedTempFile, String> {
    if !response.status().is_success() {
        return Err(format!(
            "Addon download returned HTTP {}.",
            response.status().as_u16()
        ));
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_ARCHIVE_BYTES)
    {
        return Err("The plugin ZIP exceeds 128 MiB.".into());
    }
    let temporary = tempfile::Builder::new()
        .prefix("godot-hub-plugin-")
        .suffix(".zip")
        .tempfile()
        .map_err(|error| format!("Cannot create a temporary plugin ZIP: {error}"))?;
    let file = temporary
        .reopen()
        .map_err(|error| format!("Cannot write the downloaded plugin ZIP: {error}"))?;
    let mut writer = tokio::io::BufWriter::new(tokio::fs::File::from_std(file));
    let mut total = 0;
    let mut hash = checksum.map(|_| Sha256::new());
    while let Some(chunk) = response.chunk().await.map_err(|error| {
        format!(
            "The addon download was interrupted: {}",
            error.without_url()
        )
    })? {
        total += chunk.len() as u64;
        if total > MAX_ARCHIVE_BYTES {
            return Err("The plugin ZIP exceeds 128 MiB.".into());
        }
        if let Some(hash) = hash.as_mut() {
            hash.update(&chunk);
        }
        writer
            .write_all(&chunk)
            .await
            .map_err(|error| format!("Cannot save the plugin ZIP: {error}"))?;
    }
    writer
        .flush()
        .await
        .map_err(|error| format!("Cannot finish the plugin download: {error}"))?;
    if let Some((expected, actual)) = checksum.zip(hash) {
        if !format!("{:x}", actual.finalize()).eq_ignore_ascii_case(expected) {
            return Err("SHA-256 verification failed for the plugin ZIP.".into());
        }
    }
    Ok(temporary)
}

fn project_root(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() || !directory_exists(path)? {
        return Err("The Godot project folder is missing.".into());
    }
    let resolved = canonical_path(path)
        .map_err(|error| format!("Cannot resolve the Godot project folder: {error}"))?;
    if !same_path(path, &resolved) {
        return Err("The project folder resolves to a different location. Import the project from its actual folder.".into());
    }
    Ok(resolved)
}

fn directory_exists(path: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() && !archive::is_filesystem_link(&metadata) => Ok(true),
        Ok(_) => Err(format!(
            "{} must be a regular folder without a symlink or junction.",
            path.display()
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!("Cannot check {}: {error}", path.display())),
    }
}

fn ensure_directory(path: &Path) -> Result<bool, String> {
    if directory_exists(path)? {
        return Ok(false);
    }
    match fs::create_dir(path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists && directory_exists(path)? => {
            Ok(false)
        }
        Err(error) => Err(format!("Cannot create {}: {error}", path.display())),
    }
}

fn open_unlinked_file(path: &Path) -> Result<Option<File>, String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() && !archive::is_filesystem_link(&metadata) => {
            File::open(path)
                .map(Some)
                .map_err(|error| format!("Cannot open {}: {error}", path.display()))
        }
        Ok(_) => Err(format!(
            "{} must be a regular file without a symlink.",
            path.display()
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("Cannot check {}: {error}", path.display())),
    }
}

fn require_vacant(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(_) => Err(format!(
            "{} already exists. Move that folder or choose a different addon.",
            path.display()
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("Cannot check {}: {error}", path.display())),
    }
}

fn immediate_directories(directory: &Path) -> Result<Vec<PathBuf>, String> {
    let entries = fs::read_dir(directory)
        .map_err(|error| format!("Cannot read {}: {error}", directory.display()))?;
    let mut directories = Vec::new();
    for entry in entries {
        let path = entry
            .map_err(|error| format!("Cannot read {}: {error}", directory.display()))?
            .path();
        if directory_exists(&path).unwrap_or(false) {
            directories.push(path);
        }
    }
    directories.sort_unstable();
    Ok(directories)
}

fn valid_id(id: &str) -> bool {
    !id.contains(['/', '\\'])
        && id.encode_utf16().count() <= 255
        && archive::valid_windows_component(id)
}

fn addon_id(path: &Path) -> Option<&str> {
    path.file_name()?.to_str().filter(|id| valid_id(id))
}

fn package_addons(payload: &Path, zip: &Path) -> Result<Vec<(PathBuf, String)>, String> {
    let addons = payload.join(ADDONS);
    let mut directories = immediate_directories(payload)?;
    if directory_exists(&addons)? {
        directories = immediate_directories(&addons)?;
    } else {
        let wrapped = directories
            .iter()
            .map(|path| path.join(ADDONS))
            .filter(|path| path.is_dir())
            .collect::<Vec<_>>();
        match wrapped.as_slice() {
            [directory] => directories = immediate_directories(directory)?,
            [] if payload.join(PLUGIN_CONFIG).is_file() => {
                let id = zip
                    .file_stem()
                    .and_then(|name| name.to_str())
                    .filter(|id| valid_id(id))
                    .ok_or("The ZIP filename cannot be used as an addon folder name.")?;
                return Ok(vec![(payload.to_owned(), id.to_owned())]);
            }
            [] => {}
            _ => {
                return Err(
                    "The ZIP contains multiple packages with separate addons folders.".into(),
                )
            }
        }
    }
    Ok(directories
        .into_iter()
        .filter_map(|path| {
            let id = addon_id(&path)?.to_owned();
            Some((path, id))
        })
        .collect())
}

fn plugin_at(directory: &Path, id: &str) -> Result<Option<ProjectPlugin>, String> {
    let path = directory.join(PLUGIN_CONFIG);
    let Some(file) = open_unlinked_file(&path)? else {
        return Ok(None);
    };
    let body = read_text(file, &path)?;
    let mut plugin = ProjectPlugin {
        id: id.to_owned(),
        name: id.to_owned(),
        version: String::new(),
        description: String::new(),
        author: String::new(),
        directory: directory.to_owned(),
        managed: false,
    };
    let mut found = false;
    let mut in_plugin = false;
    for line in body.trim_start_matches('\u{feff}').lines().map(str::trim) {
        if line.starts_with([';', '#']) {
            continue;
        }
        if let Some((section, _)) = line.strip_prefix('[').and_then(|line| line.split_once(']')) {
            in_plugin = section.trim() == "plugin";
            found |= in_plugin;
            continue;
        }
        if !in_plugin {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let Some(value) = serde_json::Deserializer::from_str(value.trim_start())
            .into_iter::<String>()
            .next()
            .and_then(Result::ok)
        else {
            continue;
        };
        match key.trim() {
            "name" if !value.trim().is_empty() => plugin.name = value,
            "description" => plugin.description = value,
            "author" => plugin.author = value,
            "version" => plugin.version = value,
            _ => {}
        }
    }
    if !found {
        return Err(format!(
            "{} does not have a [plugin] section.",
            path.display()
        ));
    }
    Ok(Some(plugin))
}

fn read_text(file: File, path: &Path) -> Result<String, String> {
    let mut text = String::new();
    file.take(MAX_MANIFEST_BYTES + 1)
        .read_to_string(&mut text)
        .map_err(|error| format!("Cannot read {}: {error}", path.display()))?;
    if text.len() as u64 > MAX_MANIFEST_BYTES || text.contains('\0') {
        return Err(format!(
            "{} is not a supported plugin metadata file.",
            path.display()
        ));
    }
    Ok(text)
}

fn write_owner(project: &Path, directory: &Path, id: &str) -> Result<(), String> {
    let path = directory.join(OWNER);
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&path)
        .map_err(|error| format!("Cannot record ownership of addon {id}: {error}"))?;
    let owner = Owner {
        schema: 1,
        project: project.to_owned(),
        addon_id: id.into(),
    };
    serde_json::to_writer(&mut file, &owner)
        .map_err(|error| format!("Cannot save addon ownership: {error}"))?;
    file.flush()
        .map_err(|error| format!("Cannot save addon ownership: {error}"))
}

fn managed_by_hub(project: &Path, directory: &Path, id: &str) -> bool {
    let path = directory.join(OWNER);
    let Some(file) = open_unlinked_file(&path).ok().flatten() else {
        return false;
    };
    read_text(file, &path)
        .ok()
        .and_then(|text| serde_json::from_str::<Owner>(&text).ok())
        .is_some_and(|owner| {
            owner.schema == 1
                && owner.addon_id.eq_ignore_ascii_case(id)
                && same_path(&owner.project, project)
        })
}

fn create_ignore(directory: &Path) -> Result<(), String> {
    let path = directory.join(".gdignore");
    match OpenOptions::new().create_new(true).write(true).open(&path) {
        Ok(_) => Ok(()),
        Err(error)
            if error.kind() == io::ErrorKind::AlreadyExists
                && open_unlinked_file(&path)?.is_some() =>
        {
            Ok(())
        }
        Err(error) => Err(format!("Cannot prepare addon staging or backups: {error}")),
    }
}

fn same_path(first: &Path, second: &Path) -> bool {
    first == second || first.as_os_str().eq_ignore_ascii_case(second.as_os_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use zip::{write::SimpleFileOptions, ZipWriter};

    const CONFIG: &[u8] = b"[plugin]\nname=\"Example & Tools\"\nauthor=\"Fixture\"\nversion=\"1.2\"\ndescription=\"One\\nTwo\"\nscript=\"plugin.gd\"\n";

    fn zip_file(path: &Path, entries: &[(&str, &[u8])]) {
        let mut zip = ZipWriter::new(File::create(path).unwrap());
        for (name, contents) in entries {
            zip.start_file(*name, SimpleFileOptions::default()).unwrap();
            zip.write_all(contents).unwrap();
        }
        zip.finish().unwrap();
    }

    #[test]
    fn installs_wrapped_addons_only_and_preserves_local_edits_during_removal() {
        let workspace = tempfile::tempdir().unwrap();
        let project = canonical_path(workspace.path()).unwrap();
        let archive = project.join("addon.zip");
        let config = project.join("project.godot");
        fs::write(&config, b"config_version=5\n").unwrap();
        zip_file(
            &archive,
            &[
                ("repo-42/addons/example/plugin.cfg", CONFIG),
                (
                    "repo-42/addons/example/plugin.gd",
                    b"extends EditorPlugin\n",
                ),
                ("repo-42/project.godot", b"unrelated upstream project"),
            ],
        );

        let installed = install_zip(&project, &archive).unwrap();
        assert_eq!(installed.len(), 1);
        assert_eq!(installed[0].id, "example");
        assert_eq!(installed[0].description, "One\nTwo");
        assert_eq!(installed[0].author, "Fixture");
        assert!(list(&project).unwrap()[0].managed);
        assert_eq!(fs::read(&config).unwrap(), b"config_version=5\n");

        let script = installed[0].directory.join("plugin.gd");
        fs::write(&script, b"local edit").unwrap();
        assert!(install_zip(&project, &archive).is_err());
        assert_eq!(fs::read(&script).unwrap(), b"local edit");

        let backup = remove(&project, "example").unwrap();
        assert!(backup.starts_with(project.join(BACKUPS)));
        assert!(project.join(BACKUPS).join(".gdignore").is_file());
        assert_eq!(fs::read(backup.join("plugin.gd")).unwrap(), b"local edit");
        assert!(list(&project).unwrap().is_empty());
    }

    #[test]
    fn lists_manual_addons_and_does_not_move_them() {
        let workspace = tempfile::tempdir().unwrap();
        let project = canonical_path(workspace.path()).unwrap();
        let manual = project.join("addons/manual");
        fs::create_dir_all(&manual).unwrap();
        fs::write(manual.join(PLUGIN_CONFIG), CONFIG).unwrap();
        write_owner(&project.join("different project"), &manual, "manual").unwrap();

        let plugins = list(&project).unwrap();
        assert_eq!(plugins.len(), 1);
        assert_eq!(plugins[0].name, "Example & Tools");
        assert_eq!(plugins[0].version, "1.2");
        assert!(!plugins[0].managed);
        assert!(remove(&project, "manual").is_err());
        assert!(remove(&project, "../manual").is_err());
        assert!(manual.is_dir());
        assert!(!project.join(BACKUPS).exists());
    }

    #[test]
    fn rejects_bundle_conflicts_before_any_plugin_is_committed() {
        let workspace = tempfile::tempdir().unwrap();
        let project = canonical_path(workspace.path()).unwrap();
        let conflict = project.join("addons/z-existing");
        fs::create_dir_all(&conflict).unwrap();
        fs::write(conflict.join("untouched.txt"), b"keep").unwrap();
        let bundle = project.join("bundle.zip");
        zip_file(
            &bundle,
            &[
                ("addons/a-new/plugin.cfg", CONFIG),
                ("addons/z-existing/plugin.cfg", CONFIG),
            ],
        );

        assert!(install_zip(&project, &bundle).is_err());
        assert!(!project.join("addons/a-new").exists());
        assert_eq!(fs::read(conflict.join("untouched.txt")).unwrap(), b"keep");
    }

    #[test]
    fn rejects_archive_links_path_traversal_and_forged_ownership() {
        let workspace = tempfile::tempdir().unwrap();
        let project = canonical_path(workspace.path()).unwrap();
        let bad = project.join("bad.zip");
        zip_file(
            &bad,
            &[
                ("../outside.txt", b"escape"),
                ("addons/example/plugin.cfg", CONFIG),
            ],
        );
        assert!(install_zip(&project, &bad).is_err());
        assert!(!project.join("addons").exists());
        assert!(!project.join("outside.txt").exists());

        let mut linked = ZipWriter::new(File::create(&bad).unwrap());
        linked
            .start_file("addons/example/plugin.cfg", SimpleFileOptions::default())
            .unwrap();
        linked.write_all(CONFIG).unwrap();
        linked
            .add_symlink(
                "addons/example/link",
                "../outside.txt",
                SimpleFileOptions::default(),
            )
            .unwrap();
        linked.finish().unwrap();
        assert!(install_zip(&project, &bad).is_err());
        assert!(!project.join("addons").exists());

        zip_file(
            &bad,
            &[
                ("addons/example/plugin.cfg", CONFIG),
                ("addons/example/.godot-hub-plugin.json", b"ownership"),
            ],
        );
        assert!(install_zip(&project, &bad).is_err());
        assert!(!project.join("addons").exists());
    }

    #[test]
    fn serializes_live_api_field_names_and_restricts_download_urls() {
        let raw: ApiSearch = serde_json::from_str(r#"{"result":[{"asset_id":"5385","title":"DialogueDSL","author":"zHoeshin","version_string":"1.3.2","category":"Tools","cost":"MIT"}],"page":0,"page_length":12,"total_items":13}"#).unwrap();
        let search = AssetSearch {
            assets: raw.result,
            page: raw.page,
            page_count: raw.total_items.div_ceil(raw.page_length),
        };
        let json = serde_json::to_value(search).unwrap();
        assert_eq!(json["assets"][0]["id"], 5385);
        assert_eq!(json["assets"][0]["license"], "MIT");
        assert_eq!(json["pageCount"], 2);
        assert!(supported_download_url(
            &Url::parse("https://codeload.github.com/owner/repo/zip/commit").unwrap()
        ));
        for url in [
            "https://127.0.0.1/file",
            "http://github.com/a/b.zip",
            "https://github.com.invalid/a/b.zip",
            "https://user@github.com/a/b.zip",
        ] {
            assert!(!supported_download_url(&Url::parse(url).unwrap()), "{url}");
        }
    }
}
