//! OS-specific bits of the workspace host: home and cache folders, hidden-file rules, path case
//! sensitivity, and the "places" shown in the folder browser (WSL drives, Windows drives, macOS
//! volumes). Everything else in `workspace` is written once for all targets.

use std::fs;
use std::path::{Path, PathBuf};

use contextual_protocol::fs::Place;

/// Paths compare case-insensitively (default NTFS and APFS). Used by the containment checks.
pub const CASE_INSENSITIVE_PATHS: bool = cfg!(any(windows, target_os = "macos"));

/// The user's home folder (`$HOME` first on Unix, like Node's `os.homedir()`).
pub fn home_dir() -> PathBuf {
    if let Some(dirs) = directories::BaseDirs::new() {
        return dirs.home_dir().to_path_buf();
    }
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from(if cfg!(windows) { "C:\\" } else { "/" }))
}

/// Contextual's cache folder on this host: `$XDG_CACHE_HOME/contextual` or `~/.cache/contextual`
/// on Linux (as today), `~/Library/Caches/contextual` on macOS, `%LOCALAPPDATA%\contextual` on
/// Windows.
pub fn cache_dir() -> PathBuf {
    match directories::BaseDirs::new() {
        Some(dirs) => dirs.cache_dir().join("contextual"),
        None => home_dir().join(".cache").join("contextual"),
    }
}

/// Hidden entry: leading dot everywhere; on Windows also `FILE_ATTRIBUTE_HIDDEN`.
pub fn is_hidden(name: &str, entry: Option<&fs::DirEntry>) -> bool {
    if name.starts_with('.') {
        return true;
    }
    has_hidden_attribute(entry)
}

#[cfg(windows)]
fn has_hidden_attribute(entry: Option<&fs::DirEntry>) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
    entry
        .and_then(|e| e.metadata().ok())
        .is_some_and(|m| m.file_attributes() & FILE_ATTRIBUTE_HIDDEN != 0)
}

#[cfg(not(windows))]
fn has_hidden_attribute(_entry: Option<&fs::DirEntry>) -> bool {
    false
}

fn place(label: impl Into<String>, path: impl AsRef<Path>) -> Place {
    Place {
        label: label.into(),
        path: path.as_ref().to_string_lossy().into_owned(),
    }
}

/// Shortcuts for the folder browser.
///
/// - Linux: Home, `/`, and WSL's mounted Windows drives (`/mnt/c` → `C:`).
/// - macOS: Home, `/`, and `/Volumes/*`.
/// - Windows: Home and every existing drive (`C:\`).
pub fn places() -> Vec<Place> {
    let mut list = vec![place("Home", home_dir())];
    list.extend(os_places());
    list
}

#[cfg(target_os = "linux")]
fn os_places() -> Vec<Place> {
    let mut list = vec![place("/", "/")];
    // Not WSL, or /mnt is unreadable: just Home and /.
    if let Ok(rd) = fs::read_dir("/mnt") {
        let mut drives: Vec<String> = rd
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.len() == 1 && n.as_bytes()[0].is_ascii_lowercase())
            .collect();
        drives.sort();
        for d in drives {
            list.push(place(format!("{}:", d.to_uppercase()), format!("/mnt/{d}")));
        }
    }
    list
}

#[cfg(target_os = "macos")]
fn os_places() -> Vec<Place> {
    let mut list = vec![place("/", "/")];
    if let Ok(rd) = fs::read_dir("/Volumes") {
        let mut vols: Vec<String> = rd
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| !n.starts_with('.'))
            .collect();
        vols.sort();
        for v in vols {
            list.push(place(v.clone(), Path::new("/Volumes").join(&v)));
        }
    }
    list
}

#[cfg(windows)]
fn os_places() -> Vec<Place> {
    // Probing `X:\` stands in for GetLogicalDrives (no FFI in the core).
    (b'A'..=b'Z')
        .map(char::from)
        .filter(|d| fs::metadata(format!("{d}:\\")).is_ok())
        .map(|d| place(format!("{d}:"), format!("{d}:\\")))
        .collect()
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn os_places() -> Vec<Place> {
    vec![place("/", "/")]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dot_files_are_hidden() {
        assert!(is_hidden(".git", None));
        assert!(!is_hidden("src", None));
    }

    #[test]
    fn places_start_with_home() {
        let p = places();
        assert_eq!(p[0].label, "Home");
        assert_eq!(p[0].path, home_dir().to_string_lossy());
    }
}
