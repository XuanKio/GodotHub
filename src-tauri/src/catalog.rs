use reqwest::{header, redirect, Client, Response, StatusCode};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fmt, fs,
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::sync::Mutex;

const RELEASES_API: &str = "https://api.github.com/repos/godotengine/godot-builds/releases";
const RELEASES_WEB: &str = "https://github.com/godotengine/godot-builds/releases";
const CACHE_TTL: Duration = Duration::from_secs(6 * 60 * 60);
const REFRESH_DELAY: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Channel {
    Stable,
    Beta,
    Rc,
    Dev,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Edition {
    Standard,
    Dotnet,
}

impl Edition {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::Dotnet => "dotnet",
        }
    }
}

impl fmt::Display for Edition {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Asset {
    pub name: String,
    pub url: String,
    pub size: u64,
    pub digest: Option<String>,
    pub checksum_url: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Build {
    pub edition: Edition,
    pub arch: String,
    pub engine: Asset,
    pub templates: Option<Asset>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Release {
    pub tag: String,
    pub channel: Channel,
    pub notes: String,
    pub published_at: String,
    pub url: String,
    pub builds: Vec<Build>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogResult {
    pub releases: Vec<Release>,
    pub stale: bool,
    pub warning: Option<String>,
}

#[derive(Clone, Deserialize, Serialize)]
struct CachedPage {
    etag: Option<String>,
    has_next: bool,
    releases: Vec<Release>,
}

#[derive(Deserialize, Serialize)]
struct CachedCatalog {
    refreshed_at: u64,
    pages: Vec<CachedPage>,
}

impl CachedCatalog {
    fn result(&self, warning: Option<String>) -> CatalogResult {
        let mut tags = HashSet::new();
        let releases = self
            .pages
            .iter()
            .flat_map(|page| page.releases.iter())
            .filter(|release| tags.insert(release.tag.as_str()))
            .cloned()
            .collect();
        CatalogResult {
            releases,
            stale: warning.is_some(),
            warning,
        }
    }
}

#[derive(Default)]
struct CatalogState {
    read_cache: bool,
    cache: Option<CachedCatalog>,
    retry: Option<(Instant, String)>,
}

pub struct Catalog {
    http: Client,
    cache_file: PathBuf,
    state: Mutex<CatalogState>,
}

impl Catalog {
    pub fn new(cache_dir: PathBuf) -> Result<Self, String> {
        let http = Client::builder()
            .user_agent(concat!("Godot-Hub/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(12))
            .timeout(Duration::from_secs(35))
            .redirect(redirect::Policy::none())
            .build()
            .map_err(|error| format!("Could not initialize GitHub requests: {error}"))?;
        Ok(Self {
            http,
            cache_file: cache_dir.join("github-releases-v1.json"),
            state: Mutex::new(CatalogState::default()),
        })
    }

    pub async fn releases(&self, force: bool) -> Result<CatalogResult, String> {
        let mut state = self.state.lock().await;
        if !state.read_cache {
            let cache_file = self.cache_file.clone();
            state.cache = tokio::task::spawn_blocking(move || {
                let file = fs::File::open(cache_file).ok()?;
                let reader = std::io::BufReader::new(file);
                let cached: CachedCatalog = serde_json::from_reader(reader).ok()?;
                cached
                    .pages
                    .iter()
                    .any(|page| !page.releases.is_empty())
                    .then_some(cached)
            })
            .await
            .ok()
            .flatten();
            state.read_cache = true;
        }

        if let Some((until, reason)) = &state.retry {
            if Instant::now() < *until {
                return cached_or_error(&state, reason.clone());
            }
        }

        if !force {
            if let Some(cache) = &state.cache {
                let age = unix_now().checked_sub(cache.refreshed_at);
                if age.is_some_and(|seconds| seconds < CACHE_TTL.as_secs()) {
                    return Ok(cache.result(None));
                }
            }
        }

        match self.refresh(state.cache.as_ref()).await {
            Ok(cache) => {
                let result = cache.result(None);
                let cache_file = self.cache_file.clone();
                let retained = tokio::task::spawn_blocking(move || {
                    if let Err(error) = store_cache(&cache_file, &cache) {
                        eprintln!("Could not save the GitHub release catalogue: {error}");
                    }
                    cache
                })
                .await
                .map_err(|error| format!("Could not save the GitHub release catalogue: {error}"))?;
                state.cache = Some(retained);
                state.retry = None;
                Ok(result)
            }
            Err(error) => {
                state.retry = Some((Instant::now() + error.retry_after, error.message.clone()));
                cached_or_error(&state, error.message)
            }
        }
    }

    async fn refresh(
        &self,
        old_cache: Option<&CachedCatalog>,
    ) -> Result<CachedCatalog, RefreshError> {
        let mut pages = Vec::new();
        for page_number in 1..=100 {
            let previous = old_cache.and_then(|cache| cache.pages.get(page_number - 1));
            let mut request = self
                .http
                .get(RELEASES_API)
                .header(header::ACCEPT, "application/vnd.github+json")
                .header("x-github-api-version", "2022-11-28")
                .query(&[("per_page", 100), ("page", page_number)]);
            if let Some(etag) = previous.and_then(|page| page.etag.as_deref()) {
                request = request.header(header::IF_NONE_MATCH, etag);
            }

            let response = request.send().await.map_err(|error| {
                RefreshError::new(format!("Could not reach GitHub Releases: {error}"))
            })?;
            let page = if response.status() == StatusCode::NOT_MODIFIED {
                previous.cloned().ok_or_else(|| {
                    RefreshError::new("GitHub returned a cache response without local data.".into())
                })?
            } else {
                if !response.status().is_success() {
                    return Err(RefreshError::http(&response));
                }
                let etag = header_value(&response, header::ETAG.as_str()).map(str::to_owned);
                let has_next = response
                    .headers()
                    .get_all(header::LINK)
                    .iter()
                    .filter_map(|value| value.to_str().ok())
                    .any(|value| value.contains("rel=\"next\""));
                let releases: Vec<GitHubRelease> = response.json().await.map_err(|error| {
                    RefreshError::new(format!(
                        "GitHub returned unreadable release metadata: {error}"
                    ))
                })?;
                CachedPage {
                    etag,
                    has_next,
                    releases: releases
                        .into_iter()
                        .filter_map(Release::from_github)
                        .collect(),
                }
            };
            let has_next = page.has_next;
            pages.push(page);
            if !has_next {
                return Ok(CachedCatalog {
                    refreshed_at: unix_now(),
                    pages,
                });
            }
        }
        Err(RefreshError::new(
            "GitHub release pagination exceeded 100 pages.".into(),
        ))
    }
}

fn cached_or_error(state: &CatalogState, message: String) -> Result<CatalogResult, String> {
    match &state.cache {
        Some(cache) => Ok(cache.result(Some(message))),
        None => Err(message),
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn store_cache(path: &Path, cache: &CachedCatalog) -> Result<(), Box<dyn std::error::Error>> {
    let directory = path.parent().ok_or("Missing cache directory")?;
    fs::create_dir_all(directory)?;
    let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
    {
        let mut writer = BufWriter::new(temporary.as_file_mut());
        serde_json::to_writer(&mut writer, cache)?;
        writer.flush()?;
    }
    temporary.as_file().sync_all()?;
    temporary.persist(path)?;
    Ok(())
}

struct RefreshError {
    message: String,
    retry_after: Duration,
}

impl RefreshError {
    fn new(message: String) -> Self {
        Self {
            message,
            retry_after: REFRESH_DELAY,
        }
    }

    fn http(response: &Response) -> Self {
        let status = response.status();
        let retry_after =
            header_value(response, "retry-after").and_then(|value| value.parse().ok());
        let exhausted = header_value(response, "x-ratelimit-remaining") == Some("0");
        if status == StatusCode::TOO_MANY_REQUESTS || status == StatusCode::FORBIDDEN {
            let reset_delay = exhausted
                .then(|| {
                    header_value(response, "x-ratelimit-reset")?
                        .parse::<u64>()
                        .ok()
                })
                .flatten()
                .and_then(|reset| reset.checked_sub(unix_now()));
            let seconds = retry_after.or(reset_delay).unwrap_or(60).clamp(1, 3600);
            let message = if status == StatusCode::TOO_MANY_REQUESTS
                || exhausted
                || retry_after.is_some()
            {
                format!("GitHub's rate limit was reached. Try again in {seconds} seconds.")
            } else {
                format!("GitHub rejected the release catalogue request (HTTP 403). Try again in {seconds} seconds.")
            };
            return Self {
                message,
                retry_after: Duration::from_secs(seconds),
            };
        }
        Self::new(format!(
            "GitHub could not list the Godot releases (HTTP {status})."
        ))
    }
}

fn header_value<'a>(response: &'a Response, name: &str) -> Option<&'a str> {
    response.headers().get(name)?.to_str().ok()
}

#[derive(Deserialize)]
struct GitHubRelease {
    tag_name: String,
    draft: bool,
    body: Option<String>,
    published_at: Option<String>,
    assets: Vec<GitHubAsset>,
}

#[derive(Deserialize)]
struct GitHubAsset {
    name: String,
    size: u64,
    digest: Option<String>,
}

impl GitHubRelease {
    fn asset(&self, name: &str, checksum_url: Option<&str>) -> Option<Asset> {
        let asset = self
            .assets
            .iter()
            .find(|asset| asset.name == name && asset.size > 0)?;
        Some(Asset {
            name: asset.name.clone(),
            url: format!("{RELEASES_WEB}/download/{}/{name}", self.tag_name),
            size: asset.size,
            digest: asset.digest.clone(),
            checksum_url: checksum_url.map(str::to_owned),
        })
    }
}

impl Release {
    fn from_github(raw: GitHubRelease) -> Option<Self> {
        if raw.draft {
            return None;
        }
        let channel = channel(&raw.tag_name)?;
        let checksum = raw
            .asset("SHA512-SUMS.txt", None)
            .or_else(|| raw.asset("SHA256-SUMS.txt", None));
        let checksum_url = checksum.as_ref().map(|asset| asset.url.as_str());
        let prefixes = [
            format!("Godot_v{}", raw.tag_name),
            format!("Godot_v{}", raw.tag_name.replace('-', "_")),
        ];
        let find_asset = |suffix: &str| {
            prefixes
                .iter()
                .find_map(|prefix| raw.asset(&format!("{prefix}_{suffix}"), checksum_url))
        };

        let mut builds = Vec::new();
        for (edition, templates_suffix, architectures) in [
            (
                Edition::Standard,
                "export_templates.tpz",
                [
                    ("x64", "win64.exe.zip"),
                    ("arm64", "windows_arm64.exe.zip"),
                    ("x86", "win32.exe.zip"),
                ],
            ),
            (
                Edition::Dotnet,
                "mono_export_templates.tpz",
                [
                    ("x64", "mono_win64.zip"),
                    ("arm64", "mono_windows_arm64.zip"),
                    ("x86", "mono_win32.zip"),
                ],
            ),
        ] {
            let templates = find_asset(templates_suffix);
            for (arch, suffix) in architectures {
                if let Some(engine) = find_asset(suffix) {
                    builds.push(Build {
                        edition,
                        arch: arch.into(),
                        engine,
                        templates: templates.clone(),
                    });
                }
            }
        }
        if builds.is_empty() {
            return None;
        }

        Some(Self {
            url: format!("{RELEASES_WEB}/tag/{}", raw.tag_name),
            tag: raw.tag_name,
            channel,
            notes: raw.body.unwrap_or_default(),
            published_at: raw.published_at.unwrap_or_default(),
            builds,
        })
    }
}

fn channel(tag: &str) -> Option<Channel> {
    let (version, status) = tag.rsplit_once('-')?;
    if !version.contains('.')
        || version
            .split('.')
            .any(|part| part.is_empty() || !part.bytes().all(|char| char.is_ascii_digit()))
    {
        return None;
    }
    match status {
        "stable" => Some(Channel::Stable),
        _ => {
            let status = status.trim_end_matches(|char: char| char.is_ascii_digit());
            match status {
                "beta" => Some(Channel::Beta),
                "rc" => Some(Channel::Rc),
                "dev" | "alpha" => Some(Channel::Dev),
                _ => None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_godot_release_channels() {
        assert_eq!(channel("4.8-beta5"), Some(Channel::Beta));
        assert_eq!(channel("4.7.2-rc1"), Some(Channel::Rc));
        assert_eq!(channel("4.7.2-stable"), Some(Channel::Stable));
        assert_eq!(channel("4.8-dev5"), Some(Channel::Dev));
        assert_eq!(channel("4.0-alpha1"), Some(Channel::Dev));
        assert_eq!(channel("2.0.4.1-stable"), Some(Channel::Stable));
        assert_eq!(channel("4.0/../other-stable"), None);
    }

    #[test]
    fn matches_old_windows_zip_without_confusing_debug_symbols() {
        let raw = GitHubRelease {
            tag_name: "2.0.4.1-stable".into(),
            draft: false,
            body: None,
            published_at: None,
            assets: [
                "Godot_v2.0.4.1_stable_win64.exe.zip",
                "Godot_v2.0.4.1_stable_win64.exe.debugsymbols.zip",
                "Godot_v2.0.4.1_stable_export_templates.tpz",
            ]
            .map(|name| GitHubAsset {
                name: name.into(),
                size: 1,
                digest: None,
            })
            .into(),
        };
        let release = Release::from_github(raw).unwrap();
        assert_eq!(release.builds.len(), 1);
        assert_eq!(release.builds[0].edition, Edition::Standard);
        assert_eq!(release.builds[0].arch, "x64");
        assert!(release.builds[0].templates.is_some());
    }
}
