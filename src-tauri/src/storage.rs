use crate::catalog::Edition;
use serde::{Deserialize, Serialize};
use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::Mutex,
};

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    Dark,
    Light,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub engine_directory: PathBuf,
    pub project_directory: PathBuf,
    pub auto_check_updates: bool,
    pub theme: Theme,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineRecord {
    pub id: String,
    pub tag: String,
    pub edition: Edition,
    pub arch: String,
    pub executable: PathBuf,
    pub managed_directory: Option<PathBuf>,
    pub installed_at: u64,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct StateData {
    pub settings: Settings,
    pub engines: Vec<EngineRecord>,
    pub default_engine_id: Option<String>,
}

pub struct Store {
    file: PathBuf,
    data: Mutex<StateData>,
}

impl Store {
    pub fn open(config_dir: &Path, defaults: Settings) -> Result<Self, String> {
        fs::create_dir_all(config_dir)
            .map_err(|e| format!("Cannot create Hub configuration folder: {e}"))?;
        let file = config_dir.join("state.json");
        let data = match fs::read(&file) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|e| format!("Cannot read {}: {e}", file.display()))?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => StateData {
                settings: defaults,
                engines: Vec::new(),
                default_engine_id: None,
            },
            Err(e) => return Err(format!("Cannot open {}: {e}", file.display())),
        };
        Ok(Self {
            file,
            data: Mutex::new(data),
        })
    }

    pub fn read<T>(&self, operation: impl FnOnce(&StateData) -> T) -> T {
        operation(&self.data.lock().expect("Hub state lock poisoned"))
    }

    pub fn update<T>(
        &self,
        operation: impl FnOnce(&mut StateData) -> Result<T, String>,
    ) -> Result<T, String> {
        let mut data = self.data.lock().expect("Hub state lock poisoned");
        let mut next = data.clone();
        let result = operation(&mut next)?;
        self.save(&next)
            .map_err(|e| format!("Cannot save {}: {e}", self.file.display()))?;
        *data = next;
        Ok(result)
    }

    fn save(&self, data: &StateData) -> io::Result<()> {
        let mut temp =
            tempfile::NamedTempFile::new_in(self.file.parent().expect("config path has a parent"))?;
        serde_json::to_writer_pretty(temp.as_file_mut(), data)?;
        temp.as_file().sync_all()?;
        temp.persist(&self.file).map_err(|error| error.error)?;
        Ok(())
    }
}

pub fn prepare_directory(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() || path.as_os_str().is_empty() {
        return Err("Choose an absolute folder path.".into());
    }
    fs::create_dir_all(path).map_err(|e| format!("Cannot create {}: {e}", path.display()))?;
    tempfile::Builder::new()
        .prefix(".godot-hub-write-")
        .tempfile_in(path)
        .map_err(|e| format!("Cannot write to {}: {e}", path.display()))?;
    canonical_path(path).map_err(|e| format!("Cannot resolve {}: {e}", path.display()))
}

pub fn canonical_path(path: &Path) -> io::Result<PathBuf> {
    let canonical = fs::canonicalize(path)?;
    #[cfg(windows)]
    {
        if let Some(path) = canonical.to_str() {
            if let Some(unc) = path.strip_prefix(r"\\?\UNC\") {
                return Ok(PathBuf::from(format!(r"\\{unc}")));
            }
            if let Some(drive) = path.strip_prefix(r"\\?\") {
                if drive.as_bytes().get(1) == Some(&b':') {
                    return Ok(PathBuf::from(drive));
                }
            }
        }
    }
    Ok(canonical)
}
