use std::{
    fs::{self, File, OpenOptions},
    io::{self, BufWriter, Read, Write},
    path::{Path, PathBuf},
};
use zip::ZipArchive;

const MAX_FILES: usize = 50_000;
const MAX_UNCOMPRESSED_BYTES: u64 = 8 * 1024 * 1024 * 1024;

pub fn extract(archive: &Path, destination: &Path) -> Result<(), String> {
    let file =
        File::open(archive).map_err(|error| format!("Could not open the archive: {error}"))?;
    extract_file(file, destination, MAX_FILES, MAX_UNCOMPRESSED_BYTES)
}

pub(crate) fn extract_file(
    file: File,
    destination: &Path,
    max_files: usize,
    max_uncompressed_bytes: u64,
) -> Result<(), String> {
    let mut zip = ZipArchive::new(file).map_err(|error| format!("Invalid ZIP archive: {error}"))?;
    if zip.is_empty() || zip.len() > max_files {
        return Err("The archive is empty or contains too many entries.".into());
    }
    fs::create_dir_all(destination)
        .map_err(|error| format!("Could not create the extraction folder: {error}"))?;
    let metadata = fs::symlink_metadata(destination)
        .map_err(|error| format!("Could not check the extraction folder: {error}"))?;
    if !metadata.is_dir() || is_filesystem_link(&metadata) {
        return Err("The extraction folder must be a regular directory.".into());
    }
    if fs::read_dir(destination)
        .map_err(|error| format!("Could not read the extraction folder: {error}"))?
        .next()
        .is_some()
    {
        return Err("The extraction folder must be empty.".into());
    }

    let mut total = 0u64;
    for index in 0..zip.len() {
        let entry = zip
            .by_index(index)
            .map_err(|error| format!("Could not read archive entry {index}: {error}"))?;
        let is_directory = entry.is_dir();
        let path = archive_path(entry.name(), is_directory)
            .ok_or_else(|| format!("Invalid path in archive entry {index}: {}", entry.name()))?;
        let kind = entry.unix_mode().unwrap_or_default() & 0o170000;
        if !matches!(
            (kind, is_directory),
            (0, _) | (0o040000, true) | (0o100000, false)
        ) {
            return Err(format!(
                "Archive entry {} is a link or special file.",
                entry.name()
            ));
        }

        let output = destination.join(path);
        if is_directory {
            fs::create_dir_all(output)
                .map_err(|error| format!("Could not create an archive folder: {error}"))?;
            continue;
        }

        let size = entry.size();
        total = total
            .checked_add(size)
            .filter(|&size| size <= max_uncompressed_bytes)
            .ok_or_else(|| {
                format!(
                    "The archive expands past the {} MiB extraction limit.",
                    max_uncompressed_bytes / (1024 * 1024)
                )
            })?;
        let parent = output
            .parent()
            .ok_or("The archive entry has no parent folder.")?;
        fs::create_dir_all(parent)
            .map_err(|error| format!("Could not create an archive folder: {error}"))?;
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&output)
            .map_err(|error| format!("Could not create {}: {error}", output.display()))?;
        let mut writer = BufWriter::with_capacity(128 * 1024, file);
        let copied = io::copy(&mut entry.take(size + 1), &mut writer)
            .map_err(|error| format!("Could not extract {}: {error}", output.display()))?;
        if copied != size {
            return Err(format!(
                "The size of {} does not match the archive metadata.",
                output.display()
            ));
        }
        writer.flush().map_err(|error| {
            format!("Could not finish extracting {}: {error}", output.display())
        })?;
    }
    Ok(())
}

pub(crate) fn is_filesystem_link(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    metadata.file_type().is_symlink()
}

fn archive_path(name: &str, is_directory: bool) -> Option<PathBuf> {
    let name = if is_directory {
        name.strip_suffix(['/', '\\'])?
    } else {
        name
    };
    let mut path = PathBuf::new();
    for part in name.split(['/', '\\']) {
        if !valid_windows_component(part) {
            return None;
        }
        path.push(part);
    }
    Some(path)
}

pub(crate) fn valid_windows_component(name: &str) -> bool {
    if name.is_empty()
        || name.ends_with(['.', ' '])
        || name
            .chars()
            .any(|char| char.is_control() || "<>:\"|?*".contains(char))
    {
        return false;
    }
    let base = name
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end()
        .to_ascii_uppercase();
    let number = base
        .strip_prefix("COM")
        .or_else(|| base.strip_prefix("LPT"));
    !matches!(
        base.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) && !number.is_some_and(|number| {
        matches!(
            number,
            "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use zip::{write::SimpleFileOptions, ZipWriter};

    #[test]
    fn rejects_windows_paths_that_escape_or_alias_device_files() {
        for name in [
            "../editor.exe",
            r"C:\editor.exe",
            r"\editor.exe",
            "a/../editor.exe",
            "a//editor.exe",
            "a/editor.exe:stream",
            "a/COM2.exe",
            "a/NUL .exe",
            "a/CONIN$",
            "a/COM²",
            "a/file. ",
        ] {
            assert!(archive_path(name, false).is_none(), "{name}");
        }
        assert_eq!(
            archive_path("templates/windows.exe", false),
            Some(PathBuf::from("templates").join("windows.exe"))
        );
        assert_eq!(
            archive_path("templates/", true),
            Some(PathBuf::from("templates"))
        );
    }

    #[test]
    fn extracts_a_nested_file_into_a_fresh_directory() {
        let workspace = tempfile::tempdir().unwrap();
        let archive = workspace.path().join("release.zip");
        let mut zip = ZipWriter::new(File::create(&archive).unwrap());
        zip.start_file("godot/editor.exe", SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"engine").unwrap();
        zip.finish().unwrap();
        let destination = workspace.path().join("output");

        extract(&archive, &destination).unwrap();

        assert_eq!(
            fs::read(destination.join("godot/editor.exe")).unwrap(),
            b"engine"
        );
        assert!(extract(&archive, &destination).is_err());
    }

    #[test]
    fn refuses_symbolic_link_entries_before_creating_their_targets() {
        let workspace = tempfile::tempdir().unwrap();
        let archive = workspace.path().join("release.zip");
        let mut zip = ZipWriter::new(File::create(&archive).unwrap());
        zip.add_symlink("link", "../outside", SimpleFileOptions::default())
            .unwrap();
        zip.finish().unwrap();
        let destination = workspace.path().join("output");

        assert!(extract(&archive, &destination).is_err());
        assert!(fs::symlink_metadata(destination.join("link")).is_err());
        assert!(!workspace.path().join("outside").exists());
    }
}
