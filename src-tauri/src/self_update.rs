use serde::Serialize;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};
use tauri_plugin_updater::{Update, UpdaterExt};

const PROGRESS_EVENT: &str = "hub://app-update";
const RELEASE_ASSETS: &str = "/XuanKio/GodotHub/releases/download/";
const PROGRESS_INTERVAL: Duration = Duration::from_millis(120);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppUpdate {
    current_version: String,
    version: String,
    notes: Option<String>,
    published_at: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "lowercase")]
enum Stage {
    Downloading,
    Installing,
}

#[derive(Clone, Serialize)]
struct Progress {
    downloaded: u64,
    total: Option<u64>,
    stage: Stage,
}

fn trusted_installer_url(url: &reqwest::Url, version: &str) -> bool {
    let release_path = format!("{RELEASE_ASSETS}v{version}/");
    url.scheme() == "https"
        && url.host_str() == Some("github.com")
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
        && url
            .path()
            .strip_prefix(&release_path)
            .is_some_and(|asset| !asset.contains('/') && asset.ends_with("-setup.exe"))
}

async fn find_update(app: &AppHandle, timeout: Duration) -> Result<Option<Update>, String> {
    let updater = app
        .updater_builder()
        .timeout(timeout)
        .configure_client(|client| {
            client
                .https_only(true)
                .connect_timeout(Duration::from_secs(12))
        })
        .build()
        .map_err(|error| format!("Cannot initialize Godot Hub updater: {error}"))?;
    let update = updater
        .check()
        .await
        .map_err(|error| format!("Cannot check the Godot Hub release feed: {error}"))?;
    if let Some(update) = &update {
        if update.signature.trim().is_empty()
            || !trusted_installer_url(&update.download_url, &update.version)
        {
            return Err(
                "The release feed did not provide a signed Godot Hub Windows installer.".into(),
            );
        }
    }
    Ok(update)
}

pub async fn check(app: &AppHandle) -> Result<Option<AppUpdate>, String> {
    Ok(find_update(app, Duration::from_secs(25))
        .await?
        .map(|update| AppUpdate {
            current_version: update.current_version,
            version: update.version,
            notes: update.body,
            published_at: update
                .raw_json
                .get("pub_date")
                .and_then(|date| date.as_str())
                .map(str::to_owned),
        }))
}

pub async fn install(app: &AppHandle, expected_version: String) -> Result<(), String> {
    let update = find_update(app, Duration::from_secs(120))
        .await?
        .ok_or_else(|| {
            "This Godot Hub update is no longer available. Check for updates again.".to_owned()
        })?;
    if update.version != expected_version {
        return Err("The available Godot Hub version changed. Check for updates again.".into());
    }

    let mut progress = Progress {
        downloaded: 0,
        total: None,
        stage: Stage::Downloading,
    };
    let _ = app.emit(PROGRESS_EVENT, &progress);
    let mut last_progress = Instant::now();
    let bytes = update
        .download(
            |chunk, total| {
                progress.downloaded += chunk as u64;
                progress.total = total;
                if last_progress.elapsed() >= PROGRESS_INTERVAL
                    || total == Some(progress.downloaded)
                {
                    let _ = app.emit(PROGRESS_EVENT, &progress);
                    last_progress = Instant::now();
                }
            },
            || {},
        )
        .await
        .map_err(|error| format!("Cannot download or verify the Godot Hub update: {error}"))?;
    progress.stage = Stage::Installing;
    let _ = app.emit(PROGRESS_EVENT, &progress);
    update
        .install(bytes)
        .map_err(|error| format!("Cannot start the Godot Hub update installer: {error}"))
}

#[cfg(test)]
mod tests {
    use super::trusted_installer_url;

    #[test]
    fn updater_accepts_only_the_announced_github_nsis_asset_path() {
        let location = "https://github.com/XuanKio/GodotHub/releases/download/v0.4.1/Godot-Hub_0.4.1_x64-setup.exe";
        assert!(trusted_installer_url(&location.parse().unwrap(), "0.4.1"));
        assert!(!trusted_installer_url(&location.parse().unwrap(), "0.4.2"));
        for other in [
            location.replace("https://", "http://"),
            location.replace("github.com", "example.org"),
            location.replace("XuanKio", "unrelated"),
            location.replace("-setup.exe", ".zip"),
        ] {
            assert!(!trusted_installer_url(&other.parse().unwrap(), "0.4.1"));
        }
    }
}
