use crate::catalog::Asset;
use reqwest::{redirect, Client, Response, Url};
use sha2::{Digest, Sha256, Sha512};
use std::{
    path::Path,
    time::{Duration, Instant},
};
use tokio::io::{AsyncWriteExt, BufWriter};

const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);
const MAX_CHECKSUM_BYTES: usize = 256 * 1024;

pub async fn fetch(
    asset: &Asset,
    destination: &Path,
    progress: impl Fn(u64, u64) + Send,
    verifying: impl FnOnce() + Send,
) -> Result<(), String> {
    let client = Client::builder()
        .user_agent(concat!("Godot-Hub/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(15))
        .read_timeout(Duration::from_secs(45))
        .redirect(redirect::Policy::custom(|attempt| {
            let url = attempt.url();
            let release_host = matches!(
                url.host_str(),
                Some("release-assets.githubusercontent.com" | "objects.githubusercontent.com")
            );
            if attempt.previous().len() < 5
                && ((release_host && secure_url(url)) || official_asset_url(url))
            {
                attempt.follow()
            } else {
                attempt.error("Unexpected redirect for an official Godot download")
            }
        }))
        .build()
        .map_err(|error| format!("Could not initialize downloads: {error}"))?;

    let url = checked_asset_url(&asset.url)?;
    if url.path_segments().and_then(|mut path| path.next_back()) != Some(asset.name.as_str()) {
        return Err("The download URL does not match the selected release asset.".into());
    }
    let mut checksum = expected_checksum(&client, &url, asset).await?;
    let mut response = get(&client, url, &asset.name, None).await?;
    if response
        .content_length()
        .is_some_and(|length| length != asset.size)
    {
        return Err(format!(
            "GitHub reported an unexpected download size for {}.",
            asset.name
        ));
    }

    let parent = destination
        .parent()
        .ok_or("The download destination has no directory.")?;
    let temporary = tempfile::NamedTempFile::new_in(parent)
        .map_err(|error| format!("Could not create the download file: {error}"))?;
    let file = temporary
        .reopen()
        .map_err(|error| format!("Could not open the download file: {error}"))?;
    let mut writer = BufWriter::with_capacity(128 * 1024, tokio::fs::File::from_std(file));
    let mut written = 0u64;
    let mut last_progress = Instant::now();
    progress(0, asset.size);

    while let Some(chunk) = response.chunk().await.map_err(|error| {
        format!(
            "The download for {} was interrupted: {}",
            asset.name,
            error.without_url()
        )
    })? {
        written = written
            .checked_add(chunk.len() as u64)
            .filter(|&length| length <= asset.size)
            .ok_or_else(|| format!("{} was larger than expected.", asset.name))?;
        if let Some(checksum) = checksum.as_mut() {
            checksum.update(&chunk);
        }
        writer
            .write_all(&chunk)
            .await
            .map_err(|error| format!("Could not save the download: {error}"))?;
        if last_progress.elapsed() >= PROGRESS_INTERVAL && written < asset.size {
            progress(written, asset.size);
            last_progress = Instant::now();
        }
    }

    progress(written, asset.size);
    verifying();
    if written != asset.size {
        return Err(format!(
            "{} is incomplete ({written} of {} bytes).",
            asset.name, asset.size
        ));
    }
    writer
        .flush()
        .await
        .map_err(|error| format!("Could not finish the download: {error}"))?;
    drop(writer);
    if let Some(checksum) = checksum {
        checksum.verify(&asset.name)?;
    }
    temporary
        .persist_noclobber(destination)
        .map_err(|error| format!("Could not publish the downloaded archive: {error}"))?;
    Ok(())
}

fn secure_url(url: &Url) -> bool {
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
}

fn official_asset_url(url: &Url) -> bool {
    secure_url(url)
        && url.host_str() == Some("github.com")
        && url
            .path()
            .starts_with("/godotengine/godot-builds/releases/download/")
}

fn checked_asset_url(url: &str) -> Result<Url, String> {
    Url::parse(url)
        .ok()
        .filter(official_asset_url)
        .ok_or_else(|| "The release does not contain an official GitHub download URL.".into())
}

async fn get(
    client: &Client,
    url: Url,
    name: &str,
    deadline: Option<Duration>,
) -> Result<Response, String> {
    let request = client.get(url);
    let request = match deadline {
        Some(deadline) => request.timeout(deadline),
        None => request,
    };
    let response = request
        .send()
        .await
        .map_err(|error| format!("Could not download {name}: {}", error.without_url()))?;
    if response.status().is_success() {
        Ok(response)
    } else {
        Err(format!(
            "GitHub could not serve {name} (HTTP {}).",
            response.status()
        ))
    }
}

enum Checksum {
    Sha256(Sha256, String),
    Sha512(Sha512, String),
}

impl Checksum {
    fn new(algorithm: &str, hash: &str) -> Option<Self> {
        if !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return None;
        }
        match (algorithm, hash.len()) {
            ("sha256", 64) => Some(Self::Sha256(Sha256::new(), hash.to_ascii_lowercase())),
            ("sha512", 128) => Some(Self::Sha512(Sha512::new(), hash.to_ascii_lowercase())),
            _ => None,
        }
    }

    fn update(&mut self, bytes: &[u8]) {
        match self {
            Self::Sha256(state, _) => state.update(bytes),
            Self::Sha512(state, _) => state.update(bytes),
        }
    }

    fn verify(self, name: &str) -> Result<(), String> {
        let (actual, expected) = match self {
            Self::Sha256(state, expected) => (format!("{:x}", state.finalize()), expected),
            Self::Sha512(state, expected) => (format!("{:x}", state.finalize()), expected),
        };
        if actual == expected {
            Ok(())
        } else {
            Err(format!(
                "Checksum verification failed for {name}. Download the file again."
            ))
        }
    }
}

async fn expected_checksum(
    client: &Client,
    asset_url: &Url,
    asset: &Asset,
) -> Result<Option<Checksum>, String> {
    if let Some(digest) = asset.digest.as_deref().filter(|value| !value.is_empty()) {
        let checksum = digest
            .split_once(':')
            .and_then(|(algorithm, hash)| Checksum::new(algorithm, hash));
        if checksum.is_some() {
            return Ok(checksum);
        }
        if asset.checksum_url.is_none() {
            return Err(format!(
                "GitHub supplied an invalid or unsupported checksum for {}.",
                asset.name
            ));
        }
    }

    let Some(url) = asset.checksum_url.as_deref() else {
        return Ok(None);
    };
    let url = checked_asset_url(url)?;
    let (asset_release, _) = asset_url
        .path()
        .rsplit_once('/')
        .ok_or("Invalid release asset URL.")?;
    let (checksum_release, file) = url
        .path()
        .rsplit_once('/')
        .ok_or("Invalid checksum file URL.")?;
    if asset_release != checksum_release {
        return Err("The checksum file belongs to another Godot release.".into());
    }
    let algorithm = match file {
        "SHA256-SUMS.txt" => "sha256",
        "SHA512-SUMS.txt" => "sha512",
        _ => return Err("The release checksum file has an unsupported format.".into()),
    };
    let mut response = get(client, url.clone(), file, Some(Duration::from_secs(30))).await?;
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| format!("Could not read {file}: {}", error.without_url()))?
    {
        if chunk.len() > MAX_CHECKSUM_BYTES - body.len() {
            return Err(format!("The release checksum file {file} is too large."));
        }
        body.extend_from_slice(&chunk);
    }
    let body =
        std::str::from_utf8(&body).map_err(|error| format!("Could not read {file}: {error}"))?;
    parse_manifest(body, &asset.name, algorithm)
        .map(Some)
        .ok_or_else(|| {
            format!(
                "The official release checksum file has no valid checksum for {}.",
                asset.name
            )
        })
}

fn parse_manifest(manifest: &str, asset: &str, algorithm: &str) -> Option<Checksum> {
    manifest.lines().find_map(|line| {
        let (hash, name) = line.split_once(char::is_whitespace)?;
        let name = name.trim_start().trim_start_matches('*');
        (name == asset)
            .then(|| Checksum::new(algorithm, hash))
            .flatten()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_godots_sha512_manifest_and_rejects_wrong_contents() {
        let asset = "Godot_v4.2-stable_win64.exe.zip";
        let hash = format!("{:x}", Sha512::digest(b"official archive"));
        let manifest = format!("{hash}  another.zip\r\n{hash} *{asset}\r\n");
        let mut result = parse_manifest(&manifest, asset, "sha512").unwrap();
        result.update(b"official archive");
        result.verify(asset).unwrap();

        let mut result = parse_manifest(&manifest, asset, "sha512").unwrap();
        result.update(b"different archive");
        assert!(result.verify(asset).is_err());
        assert!(parse_manifest(&manifest, "missing.zip", "sha512").is_none());
    }
}
