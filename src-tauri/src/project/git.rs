//! Reads Git branch and remote information straight from `.git`, without running git.
use serde::Serialize;
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

const MAX_GIT_FILE_BYTES: u64 = 256 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitInfo {
    /// Branch name, or a short commit id when HEAD is detached.
    pub branch: Option<String>,
    pub detached: bool,
    /// Web page of the `origin` remote, when it is hosted over HTTP(S) or SSH.
    pub repository_url: Option<String>,
}

pub fn inspect(project_directory: &Path) -> Option<GitInfo> {
    let git_dir = project_directory
        .ancestors()
        .find_map(|directory| git_directory(&directory.join(".git")))?;
    let (branch, detached) = match read_small(&git_dir.join("HEAD")) {
        Some(head) => parse_head(&head),
        None => (None, false),
    };
    let common = read_small(&git_dir.join("commondir"))
        .map(|relative| git_dir.join(relative.trim()))
        .unwrap_or_else(|| git_dir.clone());
    let repository_url = read_small(&common.join("config"))
        .and_then(|config| origin_url(&config))
        .and_then(|url| web_url(&url));
    Some(GitInfo {
        branch,
        detached,
        repository_url,
    })
}

fn git_directory(dot_git: &Path) -> Option<PathBuf> {
    if dot_git.is_dir() {
        return dot_git.join("HEAD").is_file().then(|| dot_git.to_owned());
    }
    let pointer = read_small(dot_git)?;
    let target = PathBuf::from(pointer.trim().strip_prefix("gitdir:")?.trim());
    let target = if target.is_absolute() {
        target
    } else {
        dot_git.parent()?.join(target)
    };
    target.join("HEAD").is_file().then_some(target)
}

fn read_small(path: &Path) -> Option<String> {
    let mut text = String::new();
    fs::File::open(path)
        .ok()?
        .take(MAX_GIT_FILE_BYTES)
        .read_to_string(&mut text)
        .ok()?;
    Some(text)
}

fn parse_head(head: &str) -> (Option<String>, bool) {
    let head = head.trim();
    if let Some(reference) = head.strip_prefix("ref:") {
        let reference = reference.trim();
        let branch = reference.strip_prefix("refs/heads/").unwrap_or(reference);
        return (Some(branch.to_owned()).filter(|b| !b.is_empty()), false);
    }
    let short = head.get(..7).filter(|id| id.chars().all(|c| c.is_ascii_hexdigit()));
    (short.map(str::to_owned), short.is_some())
}

fn origin_url(config: &str) -> Option<String> {
    let mut in_origin = false;
    for line in config.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            let section: String = line.chars().filter(|c| !c.is_whitespace()).collect();
            in_origin = section.eq_ignore_ascii_case("[remote\"origin\"]");
        } else if in_origin {
            if let Some((key, value)) = line.split_once('=') {
                if key.trim().eq_ignore_ascii_case("url") {
                    return Some(value.trim().trim_matches('"').to_owned());
                }
            }
        }
    }
    None
}

/// Turns a clone URL into the repository's web page. Only http(s) results are returned.
pub fn web_url(remote: &str) -> Option<String> {
    let remote = remote.trim();
    let (host, path) = if let Some(rest) = remote
        .strip_prefix("https://")
        .or_else(|| remote.strip_prefix("http://"))
    {
        let rest = rest.rsplit_once('@').map_or(rest, |(_, after)| after);
        rest.split_once('/')?
    } else if let Some(rest) = remote
        .strip_prefix("ssh://")
        .or_else(|| remote.strip_prefix("git://"))
    {
        let rest = rest.rsplit_once('@').map_or(rest, |(_, after)| after);
        let (host, path) = rest.split_once('/')?;
        (host.split(':').next()?, path)
    } else if !remote.contains("://") {
        let (user_host, path) = remote.split_once(':')?;
        let host = user_host.rsplit_once('@').map_or(user_host, |(_, h)| h);
        if host.len() < 2 {
            return None; // a Windows drive letter, not scp-style SSH
        }
        (host, path)
    } else {
        return None;
    };
    let path = path.trim_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let safe = |text: &str| {
        !text.is_empty()
            && text
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "-._~/:".contains(c))
    };
    if !safe(host) || !safe(path) || host.contains('/') {
        return None;
    }
    let scheme = if remote.starts_with("http://") { "http" } else { "https" };
    Some(format!("{scheme}://{host}/{path}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_remote_urls_to_web_pages() {
        for (remote, expected) in [
            ("git@github.com:XuanKio/GodotHub.git", Some("https://github.com/XuanKio/GodotHub")),
            ("https://github.com/a/b.git", Some("https://github.com/a/b")),
            ("https://user:token@gitlab.com/group/sub/repo", Some("https://gitlab.com/group/sub/repo")),
            ("ssh://git@gitlab.com:2222/group/repo.git", Some("https://gitlab.com/group/repo")),
            ("C:/repos/local.git", None),
            ("file:///C:/repos/x", None),
            ("https://evil.com/a b", None),
        ] {
            assert_eq!(web_url(remote).as_deref(), expected, "{remote}");
        }
    }

    #[test]
    fn reads_branch_detached_head_and_worktree_pointers() {
        let root = tempfile::tempdir().unwrap();
        let repo = root.path().join("repo");
        let git = repo.join(".git");
        fs::create_dir_all(git.join("worktrees/feature")).unwrap();
        fs::write(git.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        fs::write(git.join("config"), "[core]\n\tbare = false\n[remote \"origin\"]\n\turl = git@github.com:me/game.git\n").unwrap();
        let nested = repo.join("godot");
        fs::create_dir(&nested).unwrap();
        let info = inspect(&nested).unwrap();
        assert_eq!(info.branch.as_deref(), Some("main"));
        assert_eq!(info.repository_url.as_deref(), Some("https://github.com/me/game"));

        let worktree = root.path().join("wt");
        fs::create_dir(&worktree).unwrap();
        let linked = git.join("worktrees/feature");
        fs::write(linked.join("HEAD"), "0123456789abcdef0123456789abcdef01234567\n").unwrap();
        fs::write(linked.join("commondir"), "../..\n").unwrap();
        fs::write(worktree.join(".git"), format!("gitdir: {}\n", linked.display())).unwrap();
        let info = inspect(&worktree).unwrap();
        assert_eq!(info.branch.as_deref(), Some("0123456"));
        assert!(info.detached);
        assert_eq!(info.repository_url.as_deref(), Some("https://github.com/me/game"));
    }
}
