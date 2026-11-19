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
    match plain_update(wanted.is_some(), &target, &current) {
        Step::Install => {}
        Step::Newest => {
            println!("rak {current} is already installed, the newest release.");
            return Ok(());
        }
        Step::Same => {
            println!("rak {current} is already installed.");
            return Ok(());
        }
        Step::Ahead => {
            println!(
                "This rak ({current}) is newer than any release (the newest is {target}); nothing was changed. \
                 To go back to it: rak update {target}"
            );
            return Ok(());
        }
    }
    let dir = install_dir()?;
    let script = download_installer(&target)?;
    // The script goes to sh on stdin: it is never written to a shared folder
    // such as /tmp, where another user could put a file of their own in its place.
    let mut child = std::process::Command::new("sh")
        .arg("-s")
        .env("RAK_VERSION", &target)
        .env("RAK_INSTALL_DIR", &dir)
        .stdin(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not run the installer: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        stdin.write_all(script.as_bytes()).map_err(|e| format!("could not run the installer: {e}"))?;
    }
    match child.wait().map_err(|e| format!("could not run the installer: {e}"))? {
        s if s.success() => Ok(()),
        s => Err(format!("the installer stopped ({s}); this rak was left as it was")),
    }
}

#[derive(Debug, PartialEq)]
enum Step {
    Install,
    /// Asked for nothing; the running rak is the newest release.
    Newest,
    /// Asked for the version that is running.
    Same,
    /// Asked for nothing; the running rak is newer than every release (a
    /// build of unreleased work). Never replaced with an older release unasked.
    Ahead,
}

/// Pure — unit-tested. What `rak update` does: a version asked for by name is
/// installed even when older; with no version, only a newer release is.
fn plain_update(asked: bool, target: &str, current: &str) -> Step {
    match (asked, compare(target, current)) {
        (_, Ordering::Equal) if asked => Step::Same,
        (_, Ordering::Equal) => Step::Newest,
        (true, _) => Step::Install,
        (false, Ordering::Greater) => Step::Install,
        (false, Ordering::Less) => Step::Ahead,
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
    release_tags_page(100)
}

/// The newest `count` release tags (GitHub lists the newest first). The
/// daily check asks for a few: the whole list is hundreds of kilobytes.
fn release_tags_page(count: usize) -> Result<Vec<String>, String> {
    let url = format!("https://api.github.com/repos/{REPO}/releases?per_page={count}");
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

/// That release's own install.sh, as text.
fn download_installer(tag: &str) -> Result<String, String> {
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
    Ok(text)
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

// MARK: - Telling a chat that a newer release exists
//
// A chat never waits for the network to start. So the two halves are apart
// in time: the notice is read from a small file the last check left, and a
// new check runs on a thread once the prompt is up, at most once a day,
// leaving its answer for the next chat. Nothing is installed: the person
// runs `rak update`.

/// Hours between two looks at GitHub.
const CHECK_EVERY_HOURS: u64 = 24;

fn check_file() -> Option<PathBuf> {
    super::settings::config_dir().map(|dir| dir.join("update-check"))
}

/// Pure — unit-tested. The saved check: when it was made (seconds since
/// 1970) and the newest release it saw.
pub fn parse_check(text: &str) -> (u64, Option<String>) {
    let value = |key: &str| text.lines().find_map(|line| line.strip_prefix(key)).map(str::trim).filter(|v| !v.is_empty());
    (value("checked=").and_then(|v| v.parse().ok()).unwrap_or(0), value("newest=").map(str::to_string))
}

/// Pure — unit-tested. Whether `RAK_UPDATE_CHECK` turns the check off.
pub fn check_off(setting: Option<&str>) -> bool {
    matches!(setting.map(|s| s.trim().to_lowercase()).as_deref(), Some("0" | "off" | "no" | "false" | "never"))
}

/// Pure — unit-tested. Whether it is time to look again.
pub fn check_due(checked: u64, now: u64) -> bool {
    now.saturating_sub(checked) >= CHECK_EVERY_HOURS * 3600
}

/// Pure — unit-tested. The line a chat shows when the saved check saw a
/// release newer than this rak; None when there is none, or it is this one.
pub fn notice_for(newest: Option<&str>, current: &str) -> Option<String> {
    let newest = newest?;
    (compare(newest, current) == Ordering::Greater)
        .then(|| format!("rak {} is out (you have {}). Run: rak update", newest.trim_start_matches('v'), current.trim_start_matches('v')))
}

/// The notice for this chat, from the last check. Reads one small file:
/// no network, nothing to wait for.
pub fn notice() -> Option<String> {
    if check_off(super::settings::key("RAK_UPDATE_CHECK").as_deref()) {
        return None;
    }
    let (_, newest) = parse_check(&std::fs::read_to_string(check_file()?).ok()?);
    notice_for(newest.as_deref(), &format!("v{}", env!("CARGO_PKG_VERSION")))
}

/// Looks for a newer release on a thread of its own, when the last look was
/// a day ago or more, and saves what it saw for the next chat. Any failure
/// (no network, GitHub busy) saves nothing and says nothing.
pub fn check_in_background() {
    if check_off(super::settings::key("RAK_UPDATE_CHECK").as_deref()) {
        return;
    }
    let Some(file) = check_file() else { return };
    let now = rak::sessions::now_seconds();
    let (checked, _) = parse_check(&std::fs::read_to_string(&file).unwrap_or_default());
    if !check_due(checked, now) {
        return;
    }
    std::thread::spawn(move || {
        let Ok(tags) = release_tags_page(5) else { return };
        let Some(newest) = newest(&tags) else { return };
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&file, format!("checked={now}\nnewest={newest}\n"));
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_chat_is_told_of_a_newer_release_from_the_last_check() {
        assert_eq!(parse_check("checked=1700000000\nnewest=v0.1.0-alpha.46\n"), (1_700_000_000, Some("v0.1.0-alpha.46".to_string())));
        assert_eq!(parse_check(""), (0, None));
        assert_eq!(parse_check("checked=soon\nnewest=\n"), (0, None));
        // Newer: said, with both versions and what to run. The same or older: nothing.
        assert_eq!(
            notice_for(Some("v0.1.0-alpha.46"), "v0.1.0-alpha.45").as_deref(),
            Some("rak 0.1.0-alpha.46 is out (you have 0.1.0-alpha.45). Run: rak update")
        );
        assert_eq!(notice_for(Some("v0.1.0-alpha.45"), "v0.1.0-alpha.45"), None);
        assert_eq!(notice_for(Some("v0.1.0-alpha.9"), "v0.1.0-alpha.45"), None, "alpha.9 is older than alpha.45");
        assert!(notice_for(Some("v0.1.0"), "v0.1.0-alpha.45").is_some(), "the release after its alphas");
        assert_eq!(notice_for(None, "v0.1.0-alpha.45"), None);
        // Once a day; and the setting turns it off.
        let day = 24 * 3600;
        assert!(check_due(0, 1_700_000_000) && check_due(1_700_000_000 - day, 1_700_000_000));
        assert!(!check_due(1_700_000_000 - day + 1, 1_700_000_000) && !check_due(1_700_000_100, 1_700_000_000));
        for off in ["0", "off", "No", " false ", "never"] {
            assert!(check_off(Some(off)), "{off}");
        }
        assert!(!check_off(None) && !check_off(Some("")) && !check_off(Some("1")) && !check_off(Some("yes")));
    }

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
    fn a_plain_update_only_goes_forward_and_a_named_one_goes_anywhere() {
        let a25 = "v0.1.0-alpha.25";
        let a26 = "v0.1.0-alpha.26";
        assert_eq!(plain_update(false, a26, a25), Step::Install, "newer release out");
        assert_eq!(plain_update(false, a26, a26), Step::Newest);
        assert_eq!(plain_update(false, a25, a26), Step::Ahead, "a build newer than every release stays");
        assert_eq!(plain_update(true, a25, a26), Step::Install, "going back when asked by name");
        assert_eq!(plain_update(true, a26, a26), Step::Same);
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
