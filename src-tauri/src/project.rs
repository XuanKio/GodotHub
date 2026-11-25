use std::{
    io,
    path::Path,
    process::{Child, Command},
};

/// Shared process entry point for the engine launcher and future project records.
pub fn launch(executable: &Path, tag: &str, project_directory: Option<&Path>) -> io::Result<Child> {
    let mut command = Command::new(executable);
    if let Some(directory) = executable.parent() {
        command.current_dir(directory);
    }
    if let Some(directory) = project_directory {
        command.arg("--editor").arg("--path").arg(directory);
    } else {
        command.arg(if crate::engine::legacy_cli(tag) {
            "-pm"
        } else {
            "--project-manager"
        });
    }
    command.spawn()
}
