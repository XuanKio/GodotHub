#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod archive;
mod catalog;
mod download;
mod engine;
mod jobs;
mod project;
mod storage;
mod windows_host;

use catalog::{Asset, Catalog, CatalogResult, Edition};
use engine::InstalledEngine;
use jobs::{Job, Jobs, Kind, Stage};
use std::{
    io,
    path::{Path, PathBuf},
};
use storage::{EngineRecord, Settings, Store, Theme};
use tauri::{AppHandle, Manager, State};
use tauri_plugin_opener::OpenerExt;
use uuid::Uuid;

struct Hub {
    store: Result<Store, String>,
    catalog: Catalog,
    jobs: Jobs,
    godot_data: PathBuf,
}

impl Hub {
    fn store(&self) -> Result<&Store, String> {
        self.store.as_ref().map_err(Clone::clone)
    }
}

#[tauri::command]
fn get_platform() -> &'static str {
    windows_host::native_arch()
}

#[tauri::command]
fn get_settings(hub: State<'_, Hub>) -> Result<Settings, String> {
    Ok(hub.store()?.read(|data| data.settings.clone()))
}

#[tauri::command]
async fn save_settings(hub: State<'_, Hub>, mut settings: Settings) -> Result<Settings, String> {
    let store = hub.store()?;
    settings.engine_directory = storage::prepare_directory(&settings.engine_directory)?;
    settings.project_directory = storage::prepare_directory(&settings.project_directory)?;
    store.update(|state| {
        if state
            .engines
            .iter()
            .filter_map(|record| record.managed_directory.as_ref())
            .any(|path| {
                settings.engine_directory.starts_with(path)
                    || settings.project_directory.starts_with(path)
            })
        {
            return Err("Choose folders outside managed Godot installations.".into());
        }
        state.settings = settings.clone();
        Ok(settings)
    })
}

#[tauri::command]
fn list_installed(hub: State<'_, Hub>) -> Result<Vec<InstalledEngine>, String> {
    Ok(engine::list(hub.store()?, &hub.godot_data))
}

#[tauri::command]
async fn list_releases(hub: State<'_, Hub>, force: bool) -> Result<CatalogResult, String> {
    hub.catalog.releases(force).await
}

#[tauri::command]
fn list_jobs(hub: State<'_, Hub>) -> Vec<Job> {
    hub.jobs.all()
}

#[tauri::command]
async fn import_engine(
    hub: State<'_, Hub>,
    executable: PathBuf,
) -> Result<InstalledEngine, String> {
    engine::import(hub.store()?, &hub.godot_data, executable).await
}

#[tauri::command]
fn launch_engine(hub: State<'_, Hub>, id: String) -> Result<(), String> {
    let record = engine::get(hub.store()?, &id)?;
    require_executable(&record)?;
    project::launch(&record.executable, &record.tag, None)
        .map(|_| ())
        .map_err(|e| {
            format!(
                "Cannot launch Godot from {}: {e}",
                record.executable.display()
            )
        })
}

fn require_executable(record: &EngineRecord) -> Result<(), String> {
    if record.executable.is_file() {
        Ok(())
    } else {
        Err(format!(
            "Godot executable not found at {}. Reinstall or reimport the engine.",
            record.executable.display()
        ))
    }
}

fn executable_directory(record: &EngineRecord) -> Result<&Path, String> {
    record
        .executable
        .parent()
        .ok_or_else(|| "Engine executable has no parent folder.".into())
}

#[tauri::command]
fn reveal_engine(hub: State<'_, Hub>, id: String) -> Result<(), String> {
    let record = engine::get(hub.store()?, &id)?;
    windows_host::reveal(executable_directory(&record)?)
}

#[tauri::command]
fn open_release_page(app: AppHandle, tag: String) -> Result<(), String> {
    if !engine::valid_tag(&tag) {
        return Err("Invalid Godot release tag.".into());
    }
    app.opener()
        .open_url(
            format!("https://github.com/godotengine/godot-builds/releases/tag/{tag}"),
            None::<&str>,
        )
        .map_err(|e| format!("Cannot open the Godot release page: {e}"))
}

#[tauri::command]
async fn create_engine_shortcut(
    app: AppHandle,
    hub: State<'_, Hub>,
    id: String,
) -> Result<PathBuf, String> {
    let record = engine::get(hub.store()?, &id)?;
    require_executable(&record)?;
    let desktop = app
        .path()
        .desktop_dir()
        .map_err(|e| format!("Cannot locate the Desktop: {e}"))?;
    let title = engine::engine_directory_name(&record.tag, record.edition, &record.arch)?;
    let destination = desktop.join(format!("{title}.lnk"));
    windows_host::create_shortcut(&record.executable, &destination)?;
    Ok(destination)
}

#[tauri::command]
fn set_default_engine(hub: State<'_, Hub>, id: String) -> Result<(), String> {
    engine::set_default(hub.store()?, &id)
}

#[tauri::command]
async fn remove_engine(app: AppHandle, id: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let hub = app.state::<Hub>();
        let store = hub.store()?;
        let record = engine::get(store, &id)?;
        if hub.jobs.is_active(&format!("templates:{id}")) {
            return Err("Wait for this engine's export template installation to finish.".into());
        }
        engine::remove(store, &record)
    })
    .await
    .map_err(|e| e.to_string())?
}

enum Install {
    Engine {
        directory: PathBuf,
        folder: String,
        tag: String,
        edition: Edition,
        arch: String,
    },
    Templates {
        record: EngineRecord,
    },
}

#[tauri::command]
async fn install_engine(
    app: AppHandle,
    hub: State<'_, Hub>,
    tag: String,
    edition: Edition,
    arch: String,
) -> Result<Job, String> {
    let folder = engine::engine_directory_name(&tag, edition, &arch)?;
    if !windows_host::compatible(&arch) {
        return Err(format!(
            "This {arch} build is incompatible with this Windows installation."
        ));
    }
    let store = hub.store()?;
    let directory = store.read(|state| {
        if state.engines.iter().any(|record| {
            record.managed_directory.is_some() && engine::same_build(record, &tag, edition, &arch)
        }) {
            return Err("This build is already installed by Godot Hub.".to_owned());
        }
        Ok(state.settings.engine_directory.clone())
    })?;
    let release = hub
        .catalog
        .releases(false)
        .await?
        .releases
        .into_iter()
        .find(|release| release.tag == tag)
        .ok_or(
            "This release is absent from the official catalogue. Refresh releases and try again.",
        )?;
    let build = release
        .builds
        .into_iter()
        .find(|build| build.edition == edition && build.arch == arch)
        .ok_or("The selected edition and architecture are unavailable for this Godot release.")?;
    let edition_label = if edition == Edition::Standard {
        "Standard"
    } else {
        ".NET"
    };
    let job = hub.jobs.reserve(
        folder.clone(),
        format!("Godot {tag} · {edition_label} · {arch}"),
        Kind::Engine,
        build.engine.size,
    )?;
    enqueue(
        app,
        &job,
        build.engine,
        Install::Engine {
            directory,
            folder,
            tag,
            edition,
            arch,
        },
    );
    Ok(job)
}

#[tauri::command]
async fn install_templates(app: AppHandle, hub: State<'_, Hub>, id: String) -> Result<Job, String> {
    let store = hub.store()?;
    let record = engine::get(store, &id)?;
    require_executable(&record)?;
    if engine::templates_installed(&record, &hub.godot_data) {
        return Err("Export templates are already installed for this engine.".into());
    }
    let release = hub
        .catalog
        .releases(false)
        .await?
        .releases
        .into_iter()
        .find(|release| release.tag == record.tag)
        .ok_or("This version has no matching official release in the catalogue.")?;
    let templates = release
        .builds
        .into_iter()
        .find(|build| build.edition == record.edition && build.arch == record.arch)
        .and_then(|build| build.templates)
        .ok_or("This build has no matching official export templates.")?;
    let job = hub.jobs.reserve(
        format!("templates:{id}"),
        format!("Godot {} export templates", record.tag),
        Kind::Templates,
        templates.size,
    )?;
    enqueue(app, &job, templates, Install::Templates { record });
    Ok(job)
}

fn enqueue(app: AppHandle, job: &Job, asset: Asset, install: Install) {
    let id = job.id.clone();
    tauri::async_runtime::spawn(async move {
        let hub = app.state::<Hub>();
        let _slot = hub.jobs.work_slot.lock().await;
        let result = run_install(&app, &id, asset, install).await;
        hub.jobs.update(&app, &id, |job| match result {
            Ok(()) => job.stage = Stage::Completed,
            Err(error) => {
                job.stage = Stage::Failed;
                job.error = Some(error);
            }
        });
    });
}

async fn run_install(
    app: &AppHandle,
    id: &str,
    asset: Asset,
    install: Install,
) -> Result<(), String> {
    let hub = app.state::<Hub>();
    let root = match &install {
        Install::Engine { directory, .. } => storage::prepare_directory(directory)?,
        Install::Templates { record } => {
            engine::get(hub.store()?, &record.id)?;
            engine::templates_staging_root(record, &hub.godot_data)?
        }
    };
    if let Install::Engine { folder, .. } = &install {
        if root.join(folder).exists() {
            return Err(format!(
                "An engine directory already exists at {}. Move it or change the engine directory.",
                root.join(folder).display()
            ));
        }
    }
    let staging = tempfile::Builder::new()
        .prefix(".godot-hub-staging-")
        .tempdir_in(&root)
        .map_err(|e| format!("Cannot prepare installation in {}: {e}", root.display()))?;
    let archive_path = staging.path().join("asset.zip");
    let payload = staging.path().join("payload");
    hub.jobs
        .update(app, id, |job| job.stage = Stage::Downloading);
    let jobs = &hub.jobs;
    download::fetch(
        &asset,
        &archive_path,
        move |downloaded, total| {
            jobs.update(app, id, |job| {
                job.downloaded = downloaded;
                job.total = total;
            });
        },
        || jobs.update(app, id, |job| job.stage = Stage::Verifying),
    )
    .await?;
    jobs.update(app, id, |job| job.stage = Stage::Extracting);

    let app = app.clone();
    let id = id.to_owned();
    tauri::async_runtime::spawn_blocking(move || {
        let hub = app.state::<Hub>();
        let store = hub.store()?;
        let result = match install {
            Install::Engine {
                folder,
                tag,
                edition,
                arch,
                ..
            } => {
                let executable = engine::unpack_engine(&archive_path, &payload, &arch)?;
                hub.jobs
                    .update(&app, &id, |job| job.stage = Stage::Installing);
                let record = EngineRecord {
                    id: Uuid::new_v4().to_string(),
                    tag,
                    edition,
                    arch,
                    executable,
                    managed_directory: None,
                    installed_at: jobs::unix_millis(),
                };
                engine::commit_install(store, record, &payload, &root.join(folder))
            }
            Install::Templates { record } => {
                let (payload, version) =
                    engine::unpack_templates(&archive_path, &payload, &record, &asset)?;
                hub.jobs
                    .update(&app, &id, |job| job.stage = Stage::Installing);
                engine::get(store, &record.id)?;
                let destination = engine::templates_destination(&record, &hub.godot_data, &version);
                engine::commit_templates(&payload, &destination, &version, &record)
            }
        };
        drop(staging);
        result
    })
    .await
    .map_err(|e| format!("Installation worker failed: {e}"))?
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let paths = app.path();
            let local_data = paths.app_local_data_dir()?;
            let defaults = Settings {
                engine_directory: local_data.join("engines"),
                project_directory: paths.document_dir()?.join("Godot Projects"),
                auto_check_updates: true,
                theme: Theme::Dark,
            };
            app.manage(Hub {
                store: Store::open(&paths.app_config_dir()?, defaults),
                catalog: Catalog::new(local_data.join("catalog")).map_err(io::Error::other)?,
                jobs: Jobs::default(),
                godot_data: paths.data_dir()?.join("Godot"),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_platform,
            get_settings,
            save_settings,
            list_installed,
            list_releases,
            list_jobs,
            import_engine,
            launch_engine,
            reveal_engine,
            open_release_page,
            create_engine_shortcut,
            set_default_engine,
            remove_engine,
            install_engine,
            install_templates,
        ])
        .run(tauri::generate_context!())
        .expect("Could not run Godot Hub");
}
