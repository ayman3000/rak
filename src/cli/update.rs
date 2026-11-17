//! `rak update`: puts the newest release (or a chosen one) where this rak is
//! installed. It finds the version on GitHub, then runs that release's own
//! install.sh with RAK_VERSION and RAK_INSTALL_DIR set, so the download is
//! checked against its published SHA-256 the same way a first install is.

use std::cmp::Ordering;
use std::path::{Path, PathBuf};
use std::time::Duration;

const REPO: &str = "ayman3000/rak";

const USAGE: &str = "rak update             install the newest release in place of this one
rak update VERSION     install that release (v0.1.0-alpha.24, 0.1.0-alpha.24 or alpha.24)
rak update --check     say whether a newer release exists; change nothing

Your keys, settings and project files are not touched.";

pub fn run(args: &[String]) -> ! {
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("{USAGE}");
        std::process::exit(0);
    }
    let (check, wanted) = match args {
        [] => (false, None),
        [flag] if flag == "--check" => (true, None),
        [v] if !v.starts_with('-') => (false, Some(v.clone())),
        _ => {
            eprintln!("rak: unknown arguments for rak update\n\n{USAGE}");
            std::process::exit(2);
        }
    };
    std::process::exit(match update(check, wanted) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("rak update: {e}");
            1
        }
    });
}

fn update(check: bool, wanted: Option<String>) -> Result<(), String> {
    if cfg!(windows) {
        return Err(format!("there are no rak builds for Windows yet. With Rust installed: cargo install --git https://github.com/{REPO}"));
    }
    let current = format!("v{}", env!("CARGO_PKG_VERSION"));
    let tags = release_tags()?;
    let target = match &wanted {
        Some(v) => {
            let tag = normalize(v, &current);
            if !tags.contains(&tag) {
                return Err(format!("there is no release {tag}. The newest is {}.", newest(&tags).unwrap_or("none")));
            }
            tag
        }
        None => newest(&tags).ok_or("found no rak release on GitHub")?.to_string(),
    };
    if check {
        match compare(&target, &current) {
            Ordering::Greater => println!("rak {target} is out (this is {current}). Run: rak update"),
            _ => println!("rak {current} is the newest release."),
        }
        return Ok(());
    }
    if target == current {
        println!("rak {current} is already installed{}.", if wanted.is_none() { ", the newest release" } else { "" });
        return Ok(());
    }
    let dir = install_dir()?;
    let script = download_installer(&target)?;
    let status = std::process::Command::new("sh")
        .arg(&script)
        .env("RAK_VERSION", &target)
        .env("RAK_INSTALL_DIR", &dir)
        .status()
        .map_err(|e| format!("could not run the installer: {e}"));
    let _ = std::fs::remove_file(&script);
    match status? {
        s if s.success() => Ok(()),
        s => Err(format!("the installer stopped ({s}); this rak was left as it was")),
    }
}

/// The folder this rak runs from, where the new one goes.
fn install_dir() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| format!("could not tell where rak is installed: {e}"))?;
    let exe = exe.canonicalize().unwrap_or(exe);
    exe.parent().map(Path::to_path_buf).ok_or_else(|| "could not tell where rak is installed".to_string())
}

fn agent() -> ureq::Agent {
    rak::http::builder().timeout_connect(Duration::from_secs(15)).timeout_read(Duration::from_secs(60)).build()
}

/// Every release tag on GitHub, pre-releases included (rak is in alpha).
fn release_tags() -> Result<Vec<String>, String> {
    let url = format!("https://api.github.com/repos/{REPO}/releases?per_page=100");
    let releases: serde_json::Value = agent()
        .get(&url)
        .set("User-Agent", "rak")
        .set("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| format!("could not ask GitHub for rak's releases: {e}"))?
        .into_json()
        .map_err(|e| format!("unreadable answer from GitHub: {e}"))?;
    Ok(tags_of(&releases))
}

/// Pure — unit-tested. The tag names in GitHub's list of releases, drafts left out.
fn tags_of(releases: &serde_json::Value) -> Vec<String> {
    releases
        .as_array()
        .map(|list| {
            list.iter()
                .filter(|r| !r["draft"].as_bool().unwrap_or(false))
                .filter_map(|r| r["tag_name"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// That release's own install.sh, saved to a temporary file.
fn download_installer(tag: &str) -> Result<PathBuf, String> {
    let url = format!("https://raw.githubusercontent.com/{REPO}/{tag}/install.sh");
    let text = agent()
        .get(&url)
        .call()
        .map_err(|e| format!("could not download the installer for {tag}: {e}"))?
        .into_string()
        .map_err(|e| format!("could not read the installer for {tag}: {e}"))?;
    if !text.starts_with("#!") {
        return Err(format!("the installer for {tag} does not look like a script; nothing was changed"));
    }
    let path = std::env::temp_dir().join(format!("rak-install-{}.sh", std::process::id()));
    std::fs::write(&path, text).map_err(|e| format!("could not save the installer: {e}"))?;
    Ok(path)
}

/// Pure — unit-tested. "alpha.24", "0.1.0-alpha.24" or "v0.1.0-alpha.24" as a
/// tag; a bare pre-release name takes the core version of the running rak.
fn normalize(v: &str, current: &str) -> String {
    let v = v.trim();
    if v.starts_with('v') {
        v.to_string()
    } else if v.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        format!("v{v}")
    } else {
        let core = current.trim_start_matches('v').split('-').next().unwrap_or("0.0.0");
        format!("v{core}-{v}")
    }
}

/// Pure — unit-tested. The highest version among the tags.
fn newest(tags: &[String]) -> Option<&str> {
    tags.iter().map(String::as_str).max_by(|a, b| compare(a, b))
}

/// Pure — unit-tested. Orders tags like v1.2.3 and v1.2.3-alpha.10: numbers as
/// numbers (alpha.10 after alpha.9), a release after its own pre-releases.
fn compare(a: &str, b: &str) -> Ordering {
    fn parts(tag: &str) -> (Vec<u64>, Option<(String, u64)>) {
        let v = tag.trim_start_matches('v');
        let (core, pre) = match v.split_once('-') {
            Some((c, p)) => (c, Some(p)),
            None => (v, None),
        };
        let core = core.split('.').map(|n| n.parse().unwrap_or(0)).collect();
        let pre = pre.map(|p| match p.split_once('.') {
            Some((name, n)) => (name.to_string(), n.parse().unwrap_or(0)),
            None => (p.to_string(), 0),
        });
        (core, pre)
    }
    let (ca, pa) = parts(a);
    let (cb, pb) = parts(b);
    ca.cmp(&cb).then_with(|| match (pa, pb) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(x), Some(y)) => x.cmp(&y),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tags(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn versions_compare_as_numbers_and_a_release_beats_its_alphas() {
        assert_eq!(compare("v0.1.0-alpha.10", "v0.1.0-alpha.9"), Ordering::Greater);
        assert_eq!(compare("v0.1.0", "v0.1.0-alpha.25"), Ordering::Greater);
        assert_eq!(compare("v0.1.0-beta.1", "v0.1.0-alpha.25"), Ordering::Greater);
        assert_eq!(compare("v0.2.0-alpha.1", "v0.1.0"), Ordering::Greater);
        assert_eq!(compare("v0.1.0-alpha.24", "v0.1.0-alpha.24"), Ordering::Equal);
        // GitHub's own order put alpha.9 before alpha.10.
        assert_eq!(newest(&tags(&["v0.1.0-alpha.9", "v0.1.0-alpha.10", "v0.1.0-alpha.2"])), Some("v0.1.0-alpha.10"));
        assert_eq!(newest(&[]), None);
    }

    #[test]
    fn a_version_can_be_written_three_ways() {
        let current = "v0.1.0-alpha.25";
        assert_eq!(normalize("alpha.24", current), "v0.1.0-alpha.24");
        assert_eq!(normalize("0.1.0-alpha.24", current), "v0.1.0-alpha.24");
        assert_eq!(normalize("v0.1.0-alpha.24", current), "v0.1.0-alpha.24");
        assert_eq!(normalize(" 0.2.0 ", current), "v0.2.0");
    }

    #[test]
    fn drafts_are_left_out_of_the_release_list() {
        let list = json!([
            { "tag_name": "v0.1.0-alpha.25", "draft": false },
            { "tag_name": "v0.1.0-alpha.26", "draft": true },
            { "tag_name": "v0.1.0-alpha.24" }
        ]);
        assert_eq!(tags_of(&list), tags(&["v0.1.0-alpha.25", "v0.1.0-alpha.24"]));
        assert!(tags_of(&json!({ "message": "rate limited" })).is_empty());
    }
}
