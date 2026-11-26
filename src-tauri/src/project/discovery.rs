use super::{
    inspect, new_record, outside_managed_engines, read_string, same_path, Project, ProjectInfo,
};
use crate::{
    engine::OWNER_MARKER,
    storage::{canonical_path, Store},
};
use serde::Serialize;
use std::{
    collections::VecDeque,
    fs,
    io::Read,
    os::windows::fs::MetadataExt,
    path::{Path, PathBuf},
};

const MAX_DIRECTORIES: usize = 5_000;
const MAX_ENTRIES: usize = 50_000;
const MAX_DEPTH: usize = 7;
const MAX_PROJECTS: usize = 500;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryResult {
    projects: Vec<Project>,
    added: usize,
    limited: bool,
}

#[cfg(test)]
impl DiscoveryResult {
    pub fn added_count(&self) -> usize {
        self.added
    }
}

pub fn discover(
    store: &Store,
    godot_data: &Path,
    selected: Option<&Path>,
) -> Result<DiscoveryResult, String> {
    let (default_root, editors, known) = store.read(|state| {
        (
            state.settings.project_directory.clone(),
            state
                .engines
                .iter()
                .map(|engine| engine.executable.clone())
                .collect::<Vec<_>>(),
            state
                .projects
                .iter()
                .map(|project| project.project_file.clone())
                .collect::<Vec<_>>(),
        )
    });
    let mut files = Vec::new();
    if selected.is_none() {
        read_godot_recents(godot_data, &mut files);
        for editor in editors {
            if let Some(parent) = editor
                .parent()
                .filter(|parent| parent.join("_sc_").is_file() || parent.join("._sc_").is_file())
            {
                read_godot_recents(&parent.join("editor_data"), &mut files);
            }
        }
    }

    let root = selected.unwrap_or(&default_root);
    if !root.is_absolute() {
        return Err("Choose an absolute folder path to scan.".into());
    }
    let limited = match canonical_path(root) {
        Ok(path) if path.is_dir() => scan(&path, &mut files, selected.is_some())?,
        _ if selected.is_some() => {
            return Err(format!(
                "Cannot open project scan folder {}.",
                root.display()
            ))
        }
        _ => false,
    };

    let mut candidates: Vec<(PathBuf, PathBuf, ProjectInfo)> = Vec::new();
    for file in files {
        let Some(parent) = file.parent() else {
            continue;
        };
        let Some(file_name) = file.file_name() else {
            continue;
        };
        let Ok(directory) = canonical_path(parent) else {
            continue;
        };
        let file = directory.join(file_name);
        if known.iter().any(|other| same_path(other, &file))
            || candidates
                .iter()
                .any(|(other, _, _)| same_path(other, &file))
        {
            continue;
        }
        let Ok(info) = inspect(&file) else {
            continue;
        };
        candidates.push((file, directory, info));
    }

    let added = if candidates.is_empty() {
        0
    } else {
        store.update(|state| {
            let mut added = 0;
            for (file, directory, info) in candidates {
                if outside_managed_engines(state, &directory).is_err()
                    || canonical_path(&file)
                        .is_ok_and(|file| outside_managed_engines(state, &file).is_err())
                    || state
                        .projects
                        .iter()
                        .any(|project| same_path(&project.project_file, &file))
                {
                    continue;
                }
                state
                    .projects
                    .push(new_record(directory, file, None, &info));
                added += 1;
            }
            Ok(added)
        })?
    };
    Ok(DiscoveryResult {
        projects: super::list(store),
        added,
        limited,
    })
}

fn read_godot_recents(location: &Path, files: &mut Vec<PathBuf>) {
    for (name, godot_3) in [("projects.cfg", false), ("editor_settings-3.tres", true)] {
        let Ok(file) = fs::File::open(location.join(name)) else {
            continue;
        };
        let mut content = String::new();
        if file
            .take(4 * 1024 * 1024)
            .read_to_string(&mut content)
            .is_err()
        {
            continue;
        }
        for directory in recent_folders(&content, godot_3) {
            collect_manifests(&directory, files);
            if files.len() == MAX_PROJECTS {
                return;
            }
        }
    }
}

fn recent_folders(contents: &str, godot_3: bool) -> impl Iterator<Item = PathBuf> + '_ {
    contents
        .trim_start_matches('\u{feff}')
        .lines()
        .filter_map(move |line| {
            let line = line.trim();
            let path = if godot_3 {
                let (key, value) = line.split_once('=')?;
                key.trim().trim_matches('"').strip_prefix("projects/")?;
                read_string(value)?
            } else {
                let section = line.strip_prefix('[')?.strip_suffix(']')?;
                read_string(section).unwrap_or_else(|| section.to_owned())
            };
            let path = PathBuf::from(path);
            path.is_absolute().then_some(path)
        })
}

fn collect_manifests(path: &Path, files: &mut Vec<PathBuf>) {
    for name in ["project.godot", "engine.cfg"] {
        if files.len() == MAX_PROJECTS {
            return;
        }
        let file = path.join(name);
        if file.is_file() {
            files.push(file);
        }
    }
}

fn scan(root: &Path, files: &mut Vec<PathBuf>, validate_root: bool) -> Result<bool, String> {
    let mut queue = VecDeque::from([(root.to_owned(), 0)]);
    let mut visited = 0;
    let mut read_entries = 0;
    let mut limited = false;
    while let Some((path, depth)) = queue.pop_front() {
        if visited == MAX_DIRECTORIES || files.len() == MAX_PROJECTS || read_entries == MAX_ENTRIES
        {
            return Ok(true);
        }
        visited += 1;
        if path.join(OWNER_MARKER).is_file() {
            continue;
        }
        collect_manifests(&path, files);
        let entries = match fs::read_dir(&path) {
            Ok(entries) => entries,
            Err(error) if validate_root && depth == 0 => {
                return Err(format!("Cannot scan {}: {error}", path.display()))
            }
            Err(_) => continue,
        };
        for entry in entries {
            if read_entries == MAX_ENTRIES {
                return Ok(true);
            }
            read_entries += 1;
            let Ok(entry) = entry else {
                continue;
            };
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            if name.starts_with('.')
                || matches!(
                    name.as_str(),
                    "addons"
                        | "node_modules"
                        | "target"
                        | "godotsharp"
                        | "$recycle.bin"
                        | "system volume information"
                )
            {
                continue;
            }
            if !fs::symlink_metadata(entry.path())
                .is_ok_and(|metadata| metadata.is_dir() && metadata.file_attributes() & 0x0400 == 0)
            {
                continue;
            }
            if depth == MAX_DEPTH || visited + queue.len() >= MAX_DIRECTORIES {
                limited = true;
                continue;
            }
            queue.push_back((entry.path(), depth + 1));
        }
    }
    Ok(limited)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{Settings, Theme};

    #[test]
    fn reads_recent_project_formats_and_ignores_non_paths() {
        let modern = recent_folders(
            "[D:/Game/Mèo]\nfavorite=true\n[global]\n[\"C:/Game X\"]",
            false,
        )
        .collect::<Vec<_>>();
        assert_eq!(
            modern,
            [PathBuf::from("D:/Game/Mèo"), PathBuf::from("C:/Game X")]
        );
        let legacy = recent_folders(
            "[resource]\nprojects/D:::Foo = \"D:/Foo\"\nfavorite_projects/D:::Other = \"D:/Other\"",
            true,
        )
        .collect::<Vec<_>>();
        assert_eq!(legacy, [PathBuf::from("D:/Foo")]);
    }

    #[test]
    fn finds_existing_projects_without_rewriting_or_duplicating_them() {
        let base = tempfile::tempdir().unwrap();
        let projects = base.path().join("Projects");
        let normal = projects.join("Remote Project");
        let addon_demo = normal.join("addons/demo");
        fs::create_dir_all(&addon_demo).unwrap();
        let manifest = normal.join("project.godot");
        let contents = "config_version=5\n[application]\nconfig/name=\"First Project\"\n";
        fs::write(&manifest, contents).unwrap();
        fs::write(addon_demo.join("project.godot"), "config_version=5\n").unwrap();
        let godot = base.path().join("Godot");
        fs::create_dir_all(&godot).unwrap();
        fs::write(
            godot.join("projects.cfg"),
            format!("[{}]\nfavorite=true", normal.display()),
        )
        .unwrap();
        let store = Store::open(
            &base.path().join("state"),
            Settings {
                engine_directory: base.path().join("Engine"),
                project_directory: projects,
                auto_check_updates: false,
                auto_update_hub: false,
                theme: Theme::Dark,
            },
        )
        .unwrap();
        assert_eq!(discover(&store, &godot, None).unwrap().added, 1);
        assert_eq!(discover(&store, &godot, None).unwrap().added, 0);
        assert_eq!(fs::read_to_string(manifest).unwrap(), contents);
    }
}
