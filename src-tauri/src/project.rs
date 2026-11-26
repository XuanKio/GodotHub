use crate::{
    catalog::Edition,
    jobs::unix_millis,
    storage::{canonical_path, EngineRecord, ProjectRecord, StateData, Store},
};
use serde::Serialize;
use std::{
    cmp::Reverse,
    fs,
    io::{self, Read, Write},
    iter::Peekable,
    path::{Component, Path, PathBuf},
    process::{Child, Command},
    str::Lines,
};
use uuid::Uuid;

mod discovery;
mod git;
mod manage;
mod template;
pub use discovery::{discover, DiscoveryResult};
pub use manage::{relocate, repository_url, set_favorite, set_hidden};
pub use template::{Renderer, Template};

const MAX_PROJECT_CONFIG_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    #[serde(flatten)]
    record: ProjectRecord,
    missing: bool,
    git: Option<git::GitInfo>,
}

impl Project {
    fn from_disk(mut record: ProjectRecord) -> Self {
        let missing = !record.project_file.is_file();
        if !missing {
            if let Ok(info) = inspect(&record.project_file) {
                info.apply(&mut record);
            }
        }
        Self::present(record, missing)
    }

    fn present(record: ProjectRecord, missing: bool) -> Self {
        let git = (!missing).then(|| git::inspect(&record.directory)).flatten();
        Self {
            record,
            missing,
            git,
        }
    }
}

struct ProjectInfo {
    name: Option<String>,
    godot_major: Option<u8>,
    godot_minor: Option<u32>,
    requires_dotnet: bool,
}

impl ProjectInfo {
    fn apply(&self, record: &mut ProjectRecord) {
        if let Some(name) = &self.name {
            record.name.clone_from(name);
        }
        record.godot_major = self.godot_major;
        record.godot_minor = self.godot_minor;
        record.requires_dotnet = self.requires_dotnet;
    }
}

pub fn list(store: &Store) -> Vec<Project> {
    let mut projects = store.read(|state| state.projects.clone());
    projects.sort_by_key(|record| {
        Reverse((
            record.favorite,
            record.last_opened_at.unwrap_or(record.added_at),
            record.added_at,
        ))
    });
    projects.into_iter().map(Project::from_disk).collect()
}

pub fn create(
    store: &Store,
    name: &str,
    parent: &Path,
    engine_id: String,
    template: Template,
    renderer: Option<Renderer>,
) -> Result<Project, String> {
    let name = project_name(name)?;
    let parent = resolve_parent(parent)?;
    let mut created_directory = None;
    let mut created_files: Vec<(PathBuf, String)> = Vec::new();
    let result = store.update(|state| {
        let engine = available_engine(state, &engine_id)?.clone();
        let contents = new_project_contents(&name, &engine, template, renderer)?;
        outside_managed_engines(state, &parent)?;
        let parent = crate::storage::prepare_directory(&parent)?;
        outside_managed_engines(state, &parent)?;
        let directory = parent.join(&name);
        outside_managed_engines(state, &directory)?;
        if state
            .projects
            .iter()
            .any(|project| same_path(&project.directory, &directory))
        {
            return Err(
                "This folder is already registered as a project. Choose another name or location."
                    .into(),
            );
        }
        fs::create_dir(&directory).map_err(|error| {
            if error.kind() == io::ErrorKind::AlreadyExists {
                format!(
                    "A file or folder already exists at {}. Choose another name.",
                    directory.display()
                )
            } else {
                format!(
                    "Cannot create the project folder {}: {error}",
                    directory.display()
                )
            }
        })?;
        created_directory = Some(directory.clone());

        if let Some(scene) = contents.scene {
            let scene_file = directory.join(template::MAIN_SCENE);
            write_project(&scene_file, &scene)?;
            created_files.push((scene_file, scene));
        }
        let project_file = directory.join(contents.file_name);
        write_project(&project_file, &contents.project)?;
        created_files.push((project_file.clone(), contents.project));
        let info = inspect(&project_file)?;
        let mut project = new_record(directory, project_file, Some(engine_id), &info);
        pin_engine(&mut project, &engine);
        state.projects.push(project.clone());
        Ok(project)
    });

    match result {
        Ok(record) => Ok(Project::present(record, false)),
        Err(error) => {
            for (path, content) in created_files {
                if fs::metadata(&path).is_ok_and(|metadata| metadata.len() == content.len() as u64)
                {
                    if fs::read_to_string(&path).as_deref().ok() == Some(content.as_str()) {
                        let _ = fs::remove_file(path);
                    }
                }
            }
            if let Some(directory) = created_directory {
                if fs::remove_dir(&directory).is_err() && directory.exists() {
                    return Err(format!(
                        "{error} The new folder remains at {}.",
                        directory.display()
                    ));
                }
            }
            Err(error)
        }
    }
}

pub fn import(store: &Store, path: &Path) -> Result<Project, String> {
    if !path.is_absolute() || !is_project_file(path) {
        return Err("Choose a project.godot or engine.cfg file.".into());
    }
    let directory = path
        .parent()
        .ok_or("The project file has no parent folder.")?;
    let directory = canonical_path(directory).map_err(|error| {
        format!(
            "Cannot locate the project folder {}: {error}",
            directory.display()
        )
    })?;
    let project_file = directory.join(path.file_name().expect("validated project file name"));
    let actual_file = canonical_path(&project_file)
        .map_err(|error| format!("Cannot locate {}: {error}", project_file.display()))?;
    let info = inspect(&project_file)?;
    let record = store.update(|state| {
        outside_managed_engines(state, &directory)?;
        outside_managed_engines(state, &actual_file)?;
        if let Some(existing) = state
            .projects
            .iter_mut()
            .find(|project| same_path(&project.project_file, &project_file))
        {
            existing.hidden = false;
            return Ok(existing.clone());
        }
        let record = new_record(directory, project_file, None, &info);
        state.projects.push(record.clone());
        Ok(record)
    })?;
    Ok(Project::from_disk(record))
}

pub fn set_engine(store: &Store, id: &str, engine_id: Option<String>) -> Result<Project, String> {
    let current = get(store, id)?;
    let info = if engine_id.is_some() {
        Some(inspect(&current.project_file)?)
    } else {
        None
    };
    let record = store.update(|state| {
        let engine = match (&engine_id, &info) {
            (Some(engine_id), Some(info)) => {
                let engine = available_engine(state, engine_id)?;
                validate_engine(engine, info)?;
                Some(engine.clone())
            }
            _ => None,
        };
        let record = state
            .projects
            .iter_mut()
            .find(|project| project.id == id)
            .ok_or("That project is no longer registered.")?;
        record.engine_id = engine_id;
        match &engine {
            Some(engine) => pin_engine(record, engine),
            None => {
                record.engine_tag = None;
                record.engine_edition = None;
            }
        }
        if let Some(info) = info {
            info.apply(record);
        }
        Ok(record.clone())
    })?;
    Ok(Project::from_disk(record))
}

pub fn open(store: &Store, id: &str) -> Result<Project, String> {
    let current = get(store, id)?;
    let info = inspect(&current.project_file)?;
    let mut spawned = false;
    let result = store.update(|state| {
        let project = state
            .projects
            .iter()
            .find(|project| project.id == id)
            .ok_or("That project is no longer registered.")?;
        let engine = project_engine(state, project, &info)?.clone();
        launch(&engine.executable, &engine.tag, Some(&project.directory)).map_err(|error| {
            format!(
                "Cannot open {} with Godot {}: {error}",
                project.name, engine.tag
            )
        })?;
        spawned = true;

        let project = state
            .projects
            .iter_mut()
            .find(|project| project.id == id)
            .expect("the project is unchanged while holding the store lock");
        project.last_opened_at = Some(unix_millis());
        pin_engine(project, &engine);
        info.apply(project);
        Ok(project.clone())
    });
    let record = result.map_err(|error| {
        if spawned {
            format!("Godot opened the project, but Hub could not save its recent activity. {error}")
        } else {
            error
        }
    })?;
    Ok(Project::present(record, false))
}

pub fn reveal(store: &Store, id: &str) -> Result<(), String> {
    crate::windows_host::reveal(&get(store, id)?.directory)
}

pub fn launch(executable: &Path, tag: &str, project_directory: Option<&Path>) -> io::Result<Child> {
    let mut command = Command::new(executable);
    if let Some(directory) = executable.parent() {
        command.current_dir(directory);
    }
    let legacy = crate::engine::legacy_cli(tag);
    if let Some(directory) = project_directory {
        let flags = if legacy {
            ["-editor", "-path"]
        } else {
            ["--editor", "--path"]
        };
        command.args(flags).arg(directory);
    } else {
        command.arg(if legacy { "-pm" } else { "--project-manager" });
    }
    command.spawn()
}

fn get(store: &Store, id: &str) -> Result<ProjectRecord, String> {
    store
        .read(|state| {
            state
                .projects
                .iter()
                .find(|project| project.id == id)
                .cloned()
        })
        .ok_or_else(|| "That project is no longer registered.".into())
}

pub fn for_plugins(store: &Store, id: &str) -> Result<(PathBuf, Option<String>), String> {
    let record = get(store, id)?;
    let info = inspect(&record.project_file)?;
    if !same_path(
        &canonical_path(&record.directory).map_err(|e| e.to_string())?,
        &record.directory,
    ) {
        return Err(
            "The project folder has moved. Import it at its new location before managing plugins."
                .into(),
        );
    }
    let version = record
        .engine_id
        .and_then(|id| crate::engine::get(store, &id).ok())
        .and_then(|engine| {
            Some(format!(
                "{}.{}",
                engine_major(&engine)?,
                engine_minor(&engine).ok()?
            ))
        })
        .or_else(|| info.godot_major.map(|major| format!("{major}.99")));
    Ok((record.directory, version))
}

fn new_record(
    directory: PathBuf,
    project_file: PathBuf,
    engine_id: Option<String>,
    info: &ProjectInfo,
) -> ProjectRecord {
    let name = info.name.clone().unwrap_or_else(|| {
        directory
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| directory.display().to_string())
    });
    ProjectRecord {
        id: Uuid::new_v4().to_string(),
        name,
        directory,
        project_file,
        engine_id,
        added_at: unix_millis(),
        last_opened_at: None,
        godot_major: info.godot_major,
        godot_minor: info.godot_minor,
        requires_dotnet: info.requires_dotnet,
        favorite: false,
        hidden: false,
        engine_tag: None,
        engine_edition: None,
    }
}

fn pin_engine(record: &mut ProjectRecord, engine: &EngineRecord) {
    record.engine_tag = Some(engine.tag.clone());
    record.engine_edition = Some(engine.edition);
}

fn available_engine<'a>(state: &'a StateData, id: &str) -> Result<&'a EngineRecord, String> {
    let engine = state.engines.iter().find(|engine| engine.id == id).ok_or(
        "The selected Godot installation is no longer registered. Choose an installed engine.",
    )?;
    if !engine.executable.is_file() {
        return Err(format!(
            "Godot executable not found at {}. Choose an available engine.",
            engine.executable.display()
        ));
    }
    Ok(engine)
}

fn engine_major(engine: &EngineRecord) -> Option<u8> {
    engine.tag.split('.').next()?.parse().ok()
}

fn engine_minor(engine: &EngineRecord) -> Result<u32, String> {
    engine
        .tag
        .split('.')
        .nth(1)
        .and_then(|minor| minor.split('-').next()?.parse().ok())
        .ok_or_else(|| "Could not determine the selected Godot minor version.".into())
}

fn validate_engine(engine: &EngineRecord, info: &ProjectInfo) -> Result<(), String> {
    if let Some(project_major) = info.godot_major {
        if let Some(engine_major) = engine_major(engine) {
            if project_major != engine_major {
                return Err(format!("This project uses Godot {project_major}. Choose a Godot {project_major} engine."));
            }
        }
    }
    if info.requires_dotnet && engine.edition != Edition::Dotnet {
        return Err("This project uses C#. Choose a .NET (Mono) engine.".into());
    }
    Ok(())
}

fn project_engine<'a>(
    state: &'a StateData,
    project: &ProjectRecord,
    info: &ProjectInfo,
) -> Result<&'a EngineRecord, String> {
    let id = project
        .engine_id
        .as_deref()
        .ok_or("Assign a Godot engine before opening this project.")?;
    let engine = available_engine(state, id)?;
    validate_engine(engine, info)?;
    Ok(engine)
}

fn is_project_file(path: &Path) -> bool {
    path.file_name().is_some_and(|name| {
        name.eq_ignore_ascii_case("project.godot") || name.eq_ignore_ascii_case("engine.cfg")
    })
}

fn inspect(path: &Path) -> Result<ProjectInfo, String> {
    if !path.is_file() {
        return Err(format!("Project file not found at {}.", path.display()));
    }
    let mut body = String::new();
    fs::File::open(path)
        .and_then(|file| {
            file.take(MAX_PROJECT_CONFIG_BYTES + 1)
                .read_to_string(&mut body)
        })
        .map_err(|error| format!("Cannot read {}: {error}", path.display()))?;
    if body.len() as u64 > MAX_PROJECT_CONFIG_BYTES {
        return Err(format!(
            "The Godot configuration at {} exceeds 16 MiB.",
            path.display()
        ));
    }
    if body.contains('\0') {
        return Err(format!(
            "{} is not a Godot text project file.",
            path.display()
        ));
    }
    let legacy = path
        .file_name()
        .is_some_and(|name| name.eq_ignore_ascii_case("engine.cfg"));
    Ok(parse_project(&body, legacy))
}

fn parse_project(body: &str, legacy: bool) -> ProjectInfo {
    let mut section = "";
    let mut version = None;
    let mut feature_major = None;
    let mut feature_minor = None;
    let mut name = None;
    let mut requires_dotnet = false;
    let mut lines = body.trim_start_matches('\u{feff}').lines().peekable();
    while let Some(line) = lines.next() {
        let line = line.trim();
        if line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if let Some(header) = line
            .strip_prefix('[')
            .and_then(|line| line.split_once(']').map(|(header, _)| header))
        {
            section = header.trim();
            requires_dotnet |= matches!(section, "mono" | "dotnet");
        } else if let Some((key, value)) = line.split_once('=') {
            match (section, key.trim()) {
                ("", "config_version") => {
                    version = value
                        .split([';', '#'])
                        .next()
                        .and_then(|value| value.trim().parse::<u32>().ok());
                }
                ("application", "name") if legacy => name = read_string(value),
                ("application", "config/name") if !legacy => name = read_string(value),
                ("application", "config/features") => {
                    for feature in read_features(value, &mut lines) {
                        requires_dotnet |= feature.eq_ignore_ascii_case("C#");
                        if let Some((major, minor)) = feature.split_once('.') {
                            if let (Ok(major), Ok(minor)) =
                                (major.parse::<u8>(), minor.parse::<u32>())
                            {
                                feature_major = Some(major);
                                feature_minor = Some(minor);
                            }
                        }
                    }
                }
                ("", "custom_features") => {
                    requires_dotnet |= read_string(value).is_some_and(|features| {
                        features
                            .split(',')
                            .any(|feature| feature.trim().eq_ignore_ascii_case("C#"))
                    });
                }
                _ => {}
            }
        }
    }
    let godot_major = if legacy {
        Some(2)
    } else {
        match version {
            Some(1..=4) => Some(3),
            Some(5) => Some(4),
            _ => feature_major,
        }
    };
    ProjectInfo {
        name: name.filter(|name| !name.trim().is_empty()),
        godot_minor: feature_minor.filter(|_| godot_major == feature_major),
        godot_major,
        requires_dotnet,
    }
}

fn read_string(value: &str) -> Option<String> {
    serde_json::Deserializer::from_str(value.trim_start())
        .into_iter::<String>()
        .next()?
        .ok()
}

fn read_features(value: &str, lines: &mut Peekable<Lines<'_>>) -> Vec<String> {
    let Some((prefix, mut value)) = value.trim().split_once(['(', '[']) else {
        return Vec::new();
    };
    let closing = if prefix.is_empty() { ']' } else { ')' };
    let mut items = String::new();
    loop {
        if let Some((last, _)) = value.rsplit_once(closing) {
            items.push_str(last);
            break;
        }
        items.push_str(value);
        let Some(next) = lines.next_if(|line| {
            let line = line.trim_start();
            !line.starts_with('[') && !line.contains('=')
        }) else {
            return Vec::new();
        };
        value = next.trim();
        if value.starts_with(';') || value.starts_with('#') {
            value = "";
        }
    }
    serde_json::from_str(&format!("[{}]", items.trim().trim_end_matches(','))).unwrap_or_default()
}

fn new_project_contents(
    name: &str,
    engine: &EngineRecord,
    template: Template,
    renderer: Option<Renderer>,
) -> Result<template::Contents, String> {
    let major = engine_major(engine)
        .ok_or("Choose a Godot 2, 3 or 4 engine to create a project.")?;
    let minor = if major == 2 { 0 } else { engine_minor(engine)? };
    template::contents(
        name,
        major,
        minor,
        engine.edition == Edition::Dotnet,
        template,
        renderer,
    )
}

fn write_project(path: &Path, content: &str) -> Result<(), String> {
    let write = || -> io::Result<()> {
        let mut file =
            tempfile::NamedTempFile::new_in(path.parent().expect("project file has a parent"))?;
        file.write_all(content.as_bytes())?;
        file.as_file().sync_all()?;
        file.persist_noclobber(path).map_err(|error| error.error)?;
        Ok(())
    };
    write().map_err(|error| format!("Cannot create the project file {}: {error}", path.display()))
}

fn project_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    let stem = name
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end()
        .to_ascii_uppercase();
    let device_number = stem
        .strip_prefix("COM")
        .or_else(|| stem.strip_prefix("LPT"));
    if name.is_empty()
        || name.encode_utf16().count() > 255
        || name.ends_with('.')
        || name
            .chars()
            .any(|character| character.is_control() || "<>:\"/\\|?*".contains(character))
        || matches!(
            stem.as_str(),
            "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
        )
        || device_number.is_some_and(|digit| {
            matches!(
                digit,
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
        })
    {
        return Err("Enter a valid Windows folder name for the project.".into());
    }
    Ok(name.to_owned())
}

fn resolve_parent(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute()
        || path
            .components()
            .any(|component| component == Component::ParentDir)
    {
        return Err("Choose an absolute project location without '..' path segments.".into());
    }
    for ancestor in path.ancestors() {
        match canonical_path(ancestor) {
            Ok(canonical) => {
                return Ok(canonical.join(path.strip_prefix(ancestor).expect("matching ancestor")))
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("Cannot resolve {}: {error}", ancestor.display())),
        }
    }
    Err(format!(
        "Cannot locate the project location {}.",
        path.display()
    ))
}

fn outside_managed_engines(state: &StateData, path: &Path) -> Result<(), String> {
    let marked = path
        .ancestors()
        .any(|ancestor| ancestor.join(crate::engine::OWNER_MARKER).is_file());
    let registered = state
        .engines
        .iter()
        .filter_map(|engine| engine.managed_directory.as_ref())
        .any(|directory| {
            let canonical = canonical_path(directory).unwrap_or_else(|_| directory.clone());
            within_path(path, &canonical)
        });
    if marked || registered {
        return Err(
            "Choose a project location outside Godot Hub's managed engine installations.".into(),
        );
    }
    Ok(())
}

fn within_path(path: &Path, parent: &Path) -> bool {
    let mut path = path.components();
    parent.components().all(|part| {
        path.next()
            .is_some_and(|candidate| candidate.as_os_str().eq_ignore_ascii_case(part.as_os_str()))
    })
}

fn same_path(first: &Path, second: &Path) -> bool {
    first.components().count() == second.components().count() && within_path(first, second)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{Settings, Theme};

    fn settings(root: &Path) -> Settings {
        Settings {
            engine_directory: root.join("engines"),
            project_directory: root.join("projects"),
            auto_check_updates: true,
            auto_update_hub: false,
            theme: Theme::Dark,
        }
    }

    fn engine(root: &Path, id: &str, tag: &str, edition: Edition) -> EngineRecord {
        let executable = root.join(format!("{id}.exe"));
        fs::write(&executable, b"fixture, not executable").unwrap();
        EngineRecord {
            id: id.into(),
            tag: tag.into(),
            edition,
            arch: "x64".into(),
            executable,
            managed_directory: None,
            installed_at: 1,
        }
    }

    #[test]
    fn creates_compatible_godot_projects_in_new_folders_without_overwriting() {
        let root = tempfile::tempdir().unwrap();
        let defaults = settings(root.path());
        let parent = defaults.project_directory.clone();
        let store = Store::open(&root.path().join("config"), defaults).unwrap();
        store
            .update(|state| {
                state.engines = vec![
                    engine(root.path(), "two", "2.1.6-stable", Edition::Standard),
                    engine(
                        root.path(),
                        "early-three",
                        "3.0.6-stable",
                        Edition::Standard,
                    ),
                    engine(root.path(), "three", "3.6.3-stable", Edition::Dotnet),
                    engine(root.path(), "four", "4.7.2-stable", Edition::Dotnet),
                ];
                Ok(())
            })
            .unwrap();

        for (name, id, major, expected_config, dotnet) in [
            ("Legacy 2", "two", 2, "[application]", false),
            ("Legacy 3.0", "early-three", 3, "config_version=3", false),
            ("Godot 3", "three", 3, "config_version=4", true),
            ("Mèo & Bạn", "four", 4, "config_version=5", true),
        ] {
            let project = create(&store, name, &parent, id.into(), Template::Empty, None).unwrap();
            assert_eq!(project.record.name, name);
            assert_eq!(project.record.godot_major, Some(major));
            assert_eq!(project.record.requires_dotnet, dotnet);
            assert_eq!(project.record.engine_id.as_deref(), Some(id));
            assert_eq!(project.record.last_opened_at, None);
            let content = fs::read_to_string(project.record.project_file).unwrap();
            assert!(content.contains(expected_config));
            if id == "early-three" {
                assert!(!content.contains("GLES2"));
            }
        }

        let existing = parent.join("Existing");
        fs::create_dir(&existing).unwrap();
        let marker = existing.join("project.godot");
        fs::write(&marker, b"existing config").unwrap();
        assert!(create(&store, "Existing", &parent, "four".into(), Template::Empty, None).is_err());
        assert_eq!(fs::read(marker).unwrap(), b"existing config");
    }

    #[test]
    fn migrates_old_state_and_reimport_preserves_the_project_and_its_pin() {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join("config");
        fs::create_dir(&config).unwrap();
        let editor = engine(root.path(), "original", "4.4-stable", Edition::Standard);
        fs::write(config.join("state.json"), serde_json::to_vec(&serde_json::json!({
            "settings": settings(root.path()), "engines": [editor], "default_engine_id": "original",
        })).unwrap()).unwrap();
        let store = Store::open(&config, settings(root.path())).unwrap();
        assert!(list(&store).is_empty());

        let original = b"config_version=5\r\n[application]\r\nconfig/name=\"Disk name\"\r\n";
        let file = root.path().join("project.godot");
        fs::write(&file, original).unwrap();
        let first = import(&store, &file).unwrap();
        assert!(first.record.engine_id.is_none());
        let id = first.record.id;
        set_engine(&store, &id, Some("original".into())).unwrap();
        let again = import(&store, &file).unwrap();
        assert_eq!(again.record.id, id);
        assert_eq!(again.record.engine_id.as_deref(), Some("original"));
        assert_eq!(again.record.added_at, first.record.added_at);
        assert_eq!(fs::read(file).unwrap(), original);

        let reopened = Store::open(&config, settings(root.path())).unwrap();
        assert_eq!(list(&reopened)[0].record.id, id);
        assert_eq!(
            reopened
                .read(|state| state.default_engine_id.clone())
                .as_deref(),
            Some("original")
        );
    }

    #[test]
    fn validates_legacy_filenames_and_protects_managed_engine_directories() {
        let root = tempfile::tempdir().unwrap();
        let managed = root.path().join("Godot-managed");
        fs::create_dir(&managed).unwrap();
        let store = Store::open(&root.path().join("config"), settings(root.path())).unwrap();
        store
            .update(|state| {
                let mut editor = engine(root.path(), "current", "4.7-stable", Edition::Standard);
                editor.managed_directory = Some(canonical_path(&managed).unwrap());
                state.engines.push(editor);
                Ok(())
            })
            .unwrap();

        for name in [
            "..",
            "CON.txt",
            "LPT²",
            "a/b",
            "a\\b",
            "trailing.",
            "bad:name",
        ] {
            assert!(
                create(&store, name, root.path(), "current".into(), Template::Empty, None).is_err(),
                "{name}"
            );
        }
        let nested = managed.join("Future");
        assert!(create(&store, "Game", &nested, "current".into(), Template::Empty, None).is_err());
        assert!(!nested.exists());
        assert!(create(&store, "Game", Path::new("relative"), "current".into(), Template::Empty, None).is_err());
        let protected_file = managed.join("project.godot");
        fs::write(&protected_file, "config_version=5\n").unwrap();
        assert!(import(&store, &protected_file).is_err());
        let sibling = root.path().join("Godot-managed-folder");
        assert!(create(&store, "Game", &sibling, "current".into(), Template::Empty, None).is_ok());

        let installing = root.path().join("Godot-being-registered");
        fs::create_dir(&installing).unwrap();
        fs::write(installing.join(crate::engine::OWNER_MARKER), "install-job").unwrap();
        assert!(create(&store, "Game", &installing, "current".into(), Template::Empty, None).is_err());
    }

    #[test]
    fn uses_the_pin_independently_of_default_and_retains_unavailable_pins() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(&root.path().join("config"), settings(root.path())).unwrap();
        store
            .update(|state| {
                state.engines = vec![
                    engine(root.path(), "pinned", "4.4-stable", Edition::Standard),
                    engine(root.path(), "replacement", "4.5-stable", Edition::Standard),
                    engine(root.path(), "old-major", "3.6-stable", Edition::Standard),
                ];
                state.default_engine_id = Some("pinned".into());
                Ok(())
            })
            .unwrap();
        let project = create(&store, "Pinned", root.path(), "pinned".into(), Template::Empty, None)
            .unwrap()
            .record;
        store
            .update(|state| {
                state.default_engine_id = Some("replacement".into());
                Ok(())
            })
            .unwrap();
        let info = inspect(&project.project_file).unwrap();
        assert_eq!(
            store.read(|state| project_engine(state, &project, &info).unwrap().id.clone()),
            "pinned"
        );
        assert!(set_engine(&store, &project.id, Some("old-major".into())).is_err());
        assert!(open(&store, &project.id).is_err());
        assert!(get(&store, &project.id).unwrap().last_opened_at.is_none());

        store
            .update(|state| {
                state.engines.retain(|engine| engine.id != "pinned");
                Ok(())
            })
            .unwrap();
        assert!(open(&store, &project.id).is_err());
        assert_eq!(
            get(&store, &project.id).unwrap().engine_id.as_deref(),
            Some("pinned")
        );
        fs::remove_file(&project.project_file).unwrap();
        assert!(list(&store)[0].missing);
        assert!(set_engine(&store, &project.id, None)
            .unwrap()
            .record
            .engine_id
            .is_none());
    }

    #[test]
    fn sorts_by_activity_and_reads_managed_language_features() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(&root.path().join("config"), settings(root.path())).unwrap();
        let legacy = parse_project("[application]\nname=\"Legacy\"\n", true);
        let modern = parse_project("\u{feff}config_version=5\n[application]\nconfig/name=\"Bản thử\" ; note\nconfig/features=PackedStringArray(\n\"4.4\",\n\"C#\",\n\"GL Compatibility\"\n)\n", false);
        assert_eq!(legacy.godot_major, Some(2));
        assert_eq!(modern.godot_major, Some(4));
        assert_eq!(modern.name.as_deref(), Some("Bản thử"));
        assert!(modern.requires_dotnet);
        assert!(!parse_project("[application]\nconfig/name=\"C#\"\n", false).requires_dotnet);
        assert_eq!(
            parse_project("[application]\nconfig/name=\"Undeclared\"\n", false).godot_major,
            None
        );
        assert!(validate_engine(
            &engine(root.path(), "standard", "4.6-stable", Edition::Standard),
            &modern
        )
        .is_err());
        assert!(validate_engine(
            &engine(root.path(), "dotnet", "4.6-stable", Edition::Dotnet),
            &modern
        )
        .is_ok());

        let mut active = new_record(
            root.path().join("active"),
            root.path().join("active/project.godot"),
            None,
            &modern,
        );
        active.id = "active".into();
        active.added_at = 10;
        active.last_opened_at = Some(25);
        let mut newer = new_record(
            root.path().join("newer"),
            root.path().join("newer/project.godot"),
            None,
            &modern,
        );
        newer.id = "newer".into();
        newer.added_at = 20;
        store
            .update(|state| {
                state.projects = vec![newer, active];
                Ok(())
            })
            .unwrap();
        assert_eq!(
            list(&store)
                .into_iter()
                .map(|project| project.record.id)
                .collect::<Vec<_>>(),
            ["active", "newer"]
        );
    }
}
