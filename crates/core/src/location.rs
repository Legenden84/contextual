//! Where a project lives, and turning what the user typed into a location + native path
//! (plan §6.6). Replaces the WSL-side translation in the old `fs/paths.ts` for the Windows app.
//!
//! Everything here is pure string logic so it is tested on every OS; [`detect_for_this_os`]
//! picks the rules for the OS the app runs on.

use serde::{Deserialize, Serialize};

/// Where a project lives. `Wsl` is only produced by Windows builds.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Location {
    Local,
    Wsl { distro: String },
}

impl Location {
    /// Short label for the status bar badge: "Local" or "WSL: Ubuntu".
    pub fn label(&self) -> String {
        match self {
            Location::Local => "Local".into(),
            Location::Wsl { distro } => format!("WSL: {distro}"),
        }
    }
}

/// A project root on a host: the host's location and the path in that host's own syntax.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectRoot {
    pub location: Location,
    pub path: String,
}

/// Strips whitespace and one pair of surrounding quotes, as pasted from Explorer ("Copy as path").
fn clean(input: &str) -> &str {
    let p = input.trim();
    let p = p.strip_prefix(['"', '\'']).unwrap_or(p);
    p.strip_suffix(['"', '\'']).unwrap_or(p)
}

/// `\\wsl$\Ubuntu\home\me` / `\\wsl.localhost\Ubuntu\home\me` (either slash) → ("Ubuntu", "/home/me").
pub fn parse_wsl_unc(p: &str) -> Option<(String, String)> {
    let norm = p.replace('/', "\\");
    let rest = norm.strip_prefix("\\\\")?;
    let (host, rest) = rest.split_once('\\').unwrap_or((rest, ""));
    if !host.eq_ignore_ascii_case("wsl$") && !host.eq_ignore_ascii_case("wsl.localhost") {
        return None;
    }
    let (distro, tail) = rest.split_once('\\').unwrap_or((rest, ""));
    if distro.is_empty() {
        return None;
    }
    let tail: Vec<&str> = tail.split('\\').filter(|s| !s.is_empty()).collect();
    Some((distro.to_string(), format!("/{}", tail.join("/"))))
}

/// `/mnt/c/src/x` → `C:\src\x`.
pub fn mnt_to_windows(p: &str) -> Option<String> {
    let rest = p.strip_prefix("/mnt/")?;
    let (drive, tail) = rest.split_once('/').unwrap_or((rest, ""));
    let mut chars = drive.chars();
    let letter = chars.next().filter(|c| c.is_ascii_alphabetic())?;
    if chars.next().is_some() {
        return None;
    }
    let tail = tail.trim_end_matches('/').replace('/', "\\");
    Some(format!("{}:\\{tail}", letter.to_ascii_uppercase()))
}

/// `C:\src\x` or `c:/src/x` → `/mnt/c/src/x` (for the WSL side).
pub fn windows_to_mnt(p: &str) -> Option<String> {
    let mut chars = p.chars();
    let letter = chars.next().filter(|c| c.is_ascii_alphabetic())?;
    if chars.next() != Some(':') {
        return None;
    }
    let tail = &p[2..];
    if !(tail.is_empty() || tail.starts_with(['\\', '/'])) {
        return None;
    }
    let tail: Vec<&str> = tail.split(['\\', '/']).filter(|s| !s.is_empty()).collect();
    Some(format!("/mnt/{}/{}", letter.to_ascii_lowercase(), tail.join("/")))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Detected {
    Root(ProjectRoot),
    /// A POSIX path but WSL isn't installed (or has no default distro).
    NeedsWsl(String),
}

/// Rules for the Windows app. `default_distro` is `None` when WSL isn't installed.
/// `~` and relative paths are left for the caller's normaliser (they're local).
pub fn detect_windows(input: &str, default_distro: Option<&str>) -> Detected {
    let p = clean(input);
    if let Some((distro, path)) = parse_wsl_unc(p) {
        if let Some(win) = mnt_to_windows(&path) {
            return Detected::Root(ProjectRoot { location: Location::Local, path: win });
        }
        return Detected::Root(ProjectRoot { location: Location::Wsl { distro }, path });
    }
    if p.starts_with('/') {
        if let Some(win) = mnt_to_windows(p) {
            return Detected::Root(ProjectRoot { location: Location::Local, path: win });
        }
        return match default_distro {
            Some(d) => Detected::Root(ProjectRoot { location: Location::Wsl { distro: d.to_string() }, path: p.to_string() }),
            None => Detected::NeedsWsl(p.to_string()),
        };
    }
    Detected::Root(ProjectRoot { location: Location::Local, path: p.to_string() })
}

/// Rules for macOS (and the Linux agent): native paths only.
pub fn detect_posix(input: &str) -> Detected {
    Detected::Root(ProjectRoot { location: Location::Local, path: clean(input).to_string() })
}

pub fn detect_for_this_os(input: &str, default_distro: Option<&str>) -> Detected {
    if cfg!(windows) { detect_windows(input, default_distro) } else { detect_posix(input) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(d: Detected) -> ProjectRoot {
        match d {
            Detected::Root(r) => r,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn unc_paths() {
        assert_eq!(parse_wsl_unc(r"\\wsl.localhost\Ubuntu\home\me\x"), Some(("Ubuntu".into(), "/home/me/x".into())));
        assert_eq!(parse_wsl_unc(r"\\WSL$\Debian"), Some(("Debian".into(), "/".into())));
        assert_eq!(parse_wsl_unc("//wsl$/Ubuntu/home/"), Some(("Ubuntu".into(), "/home".into())));
        assert_eq!(parse_wsl_unc(r"\\server\share\x"), None);
        assert_eq!(parse_wsl_unc(r"\\wsl$\"), None);
    }

    #[test]
    fn drive_paths() {
        assert_eq!(mnt_to_windows("/mnt/c/src/x"), Some(r"C:\src\x".into()));
        assert_eq!(mnt_to_windows("/mnt/d"), Some(r"D:\".into()));
        assert_eq!(mnt_to_windows("/mnt/wsl/x"), None);
        assert_eq!(windows_to_mnt(r"C:\src\x"), Some("/mnt/c/src/x".into()));
        assert_eq!(windows_to_mnt("c:/"), Some("/mnt/c/".into()));
        assert_eq!(windows_to_mnt("C:"), Some("/mnt/c/".into()));
        assert_eq!(windows_to_mnt("Cx"), None);
    }

    #[test]
    fn windows_detection() {
        let wsl = |d: &str| Location::Wsl { distro: d.into() };
        assert_eq!(root(detect_windows(r"C:\src\x", Some("Ubuntu"))), ProjectRoot { location: Location::Local, path: r"C:\src\x".into() });
        assert_eq!(
            root(detect_windows(r#""\\wsl.localhost\Ubuntu\home\me\x""#, None)),
            ProjectRoot { location: wsl("Ubuntu"), path: "/home/me/x".into() }
        );
        assert_eq!(root(detect_windows("/home/me/x", Some("Ubuntu"))), ProjectRoot { location: wsl("Ubuntu"), path: "/home/me/x".into() });
        assert_eq!(detect_windows("/home/me/x", None), Detected::NeedsWsl("/home/me/x".into()));
        assert_eq!(root(detect_windows("/mnt/c/src/x", None)), ProjectRoot { location: Location::Local, path: r"C:\src\x".into() });
        assert_eq!(
            root(detect_windows(r"\\wsl$\Ubuntu\mnt\c\src", None)),
            ProjectRoot { location: Location::Local, path: r"C:\src".into() }
        );
    }

    #[test]
    fn labels() {
        assert_eq!(Location::Local.label(), "Local");
        assert_eq!(Location::Wsl { distro: "Ubuntu".into() }.label(), "WSL: Ubuntu");
    }
}
