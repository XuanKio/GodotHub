//! List housekeeping: favourites, hiding, relocating moved projects and their Git remote.
use super::{
    get, git, inspect, is_project_file, outside_managed_engines, same_path, Project,
};
use crate::storage::{canonical_path, ProjectRecord, Store};
use std::path::Path;

fn edit(
    store: &Store,
    id: &str,
    change: impl FnOnce(&mut ProjectRecord),
) -> Result<Project, String> {
    let record = store.update(|state| {
        let record = state
            .projects
            .iter_mut()
            .find(|project| project.id == id)
            .ok_or("That project is no longer registered.")?;
        change(record);
        Ok(record.clone())
    })?;
    Ok(Project::from_disk(record))
}

pub fn set_favorite(store: &Store, id: &str, favorite: bool) -> Result<Project, String> {
    edit(store, id, |record| record.favorite = favorite)
}

/// Hidden projects stay registered, so scans do not add them back.
pub fn set_hidden(store: &Store, id: &str, hidden: bool) -> Result<Project, String> {
    edit(store, id, |record| record.hidden = hidden)
}

/// Points an existing entry at the project's new location, keeping its editor and favourite.
pub fn relocate(store: &Store, id: &str, path: &Path) -> Result<Project, String> {
    if !path.is_absolute() || !is_project_file(path) {
        return Err("Choose the project's project.godot or engine.cfg file.".into());
    }
    let directory = path.parent().ok_or("The project file has no parent folder.")?;
    let directory = canonical_path(directory)
        .map_err(|error| format!("Cannot locate {}: {error}", directory.display()))?;
    let project_file = directory.join(path.file_name().expect("validated project file name"));
    let actual_file = canonical_path(&project_file)
        .map_err(|error| format!("Cannot locate {}: {error}", project_file.display()))?;
    let info = inspect(&project_file)?;
    let record = store.update(|state| {
        outside_managed_engines(state, &directory)?;
        outside_managed_engines(state, &actual_file)?;
        if let Some(other) = state
            .projects
            .iter()
            .find(|project| project.id != id && same_path(&project.project_file, &project_file))
        {
            return Err(format!(
                "That folder is already in Hub as {}. Hide one of the entries instead.",
                other.name
            ));
        }
        let record = state
            .projects
            .iter_mut()
            .find(|project| project.id == id)
            .ok_or("That project is no longer registered.")?;
        record.directory = directory;
        record.project_file = project_file;
        info.apply(record);
        Ok(record.clone())
    })?;
    Ok(Project::from_disk(record))
}

/// Web page of the project's `origin` remote, read again from disk rather than trusted from the UI.
pub fn repository_url(store: &Store, id: &str) -> Result<String, String> {
    let record = get(store, id)?;
    git::inspect(&record.directory)
        .and_then(|info| info.repository_url)
        .ok_or_else(|| "This project has no GitHub, GitLab or other web remote named origin.".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{Settings, Theme};
    use std::fs;

    #[test]
    fn favourites_hides_and_relocates_without_losing_the_entry() {
        let root = tempfile::tempdir().unwrap();
        let settings = Settings {
            engine_directory: root.path().join("engines"),
            project_directory: root.path().join("projects"),
            auto_check_updates: false,
            auto_update_hub: false,
            theme: Theme::Dark,
        };
        let store = Store::open(&root.path().join("config"), settings).unwrap();
        let old = root.path().join("old");
        fs::create_dir(&old).unwrap();
        fs::write(old.join("project.godot"), "config_version=5\n[application]\nconfig/name=\"Moved\"\n").unwrap();
        let first = super::super::import(&store, &old.join("project.godot")).unwrap();
        let id = first.record.id.clone();
        let other_dir = root.path().join("other");
        fs::create_dir(&other_dir).unwrap();
        fs::write(other_dir.join("project.godot"), "config_version=5\n").unwrap();
        let other = super::super::import(&store, &other_dir.join("project.godot")).unwrap();

        assert!(set_favorite(&store, &id, true).unwrap().record.favorite);
        assert_eq!(super::super::list(&store)[0].record.id, id);

        let new = root.path().join("new");
        fs::rename(&old, &new).unwrap();
        assert!(super::super::list(&store)[0].missing);
        assert!(relocate(&store, &id, &other_dir.join("project.godot")).is_err());
        let moved = relocate(&store, &id, &new.join("project.godot")).unwrap();
        assert!(!moved.missing);
        assert!(moved.record.favorite);
        assert_eq!(moved.record.id, id);

        assert!(set_hidden(&store, &other.record.id, true).unwrap().record.hidden);
        let result = super::super::discover(&store, root.path(), Some(root.path())).unwrap();
        assert_eq!(result.added_count(), 0);
        assert!(repository_url(&store, &id).is_err());
    }
}
