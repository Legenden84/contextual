//! Folder browser for the Open folder dialog (port of `fs/browse.ts`): lists sub-directories of
//! a path on this host. The OS-specific "places" (WSL drives, Windows drives, macOS volumes) and
//! hidden-file rules live in [`crate::platform`].

use std::fs;
use std::path::Path;

use contextual_protocol::fs::{DirEntry, DirListing};

use super::paths::{normalize_input_path, path_string};
use super::scan::locale_cmp_base;
use crate::error::{Error, Result};
use crate::platform;

const MAX_DIRS: usize = 500;
/// Project markers are only checked for the first entries, to stay fast on /mnt/c.
const MAX_MARKER_CHECKS: usize = 200;
const MARKERS: [&str; 5] = [
    ".git",
    "package.json",
    "pyproject.toml",
    "Cargo.toml",
    "go.mod",
];

fn looks_like_project(dir: &Path) -> bool {
    if MARKERS.iter().any(|m| fs::metadata(dir.join(m)).is_ok()) {
        return true;
    }
    let Ok(rd) = fs::read_dir(dir) else {
        return false;
    };
    rd.filter_map(|e| e.ok()).any(|e| {
        let name = e.file_name().to_string_lossy().to_ascii_lowercase();
        name.ends_with(".sln") || name.ends_with(".csproj")
    })
}

/// Sub-folders of `input` (normalised like any typed path; `None` = home), plus places.
pub fn list_dirs(input: Option<&str>, show_hidden: bool) -> Result<DirListing> {
    let dir = normalize_input_path(input);
    let shown_dir = path_string(&dir);
    let meta = fs::metadata(&dir)
        .map_err(|_| Error::not_found(format!("Folder not found: {shown_dir}")))?;
    if !meta.is_dir() {
        return Err(Error::invalid(format!("Not a folder: {shown_dir}")));
    }
    let rd = fs::read_dir(&dir)
        .map_err(|_| Error::forbidden(format!("Cannot read folder: {shown_dir}")))?;

    let mut names: Vec<String> = rd
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            (show_hidden || !platform::is_hidden(&name, Some(&e))).then_some(name)
        })
        .collect();
    names.sort_by(|a, b| locale_cmp_base(a, b));

    let dirs = names
        .iter()
        .take(MAX_DIRS)
        .enumerate()
        .map(|(i, name)| {
            let p = dir.join(name);
            DirEntry {
                name: name.clone(),
                path: path_string(&p),
                project: i < MAX_MARKER_CHECKS && looks_like_project(&p),
            }
        })
        .collect();

    let parent = dir.parent().filter(|p| *p != dir).map(path_string);
    Ok(DirListing {
        path: shown_dir,
        parent,
        dirs,
        places: platform::places(),
        truncated: names.len() > MAX_DIRS,
    })
}

/// A file in the env-file picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    pub name: String,
    pub path: String,
    /// Looks like a dotenv file (`.env`, `.env.local`, `local.env`).
    pub env: bool,
}

/// Whether a file name looks like a dotenv file: `.env`, `.env.<anything>`, `<anything>.env`.
pub fn is_env_file(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n == ".env" || n.starts_with(".env.") || n.ends_with(".env")
}

/// Files of `input` for the env-file picker (`None` = home): dotenv files always (they are hidden
/// on Unix), other files only with `all` (hidden ones only with `show_hidden`).
pub fn list_files(input: Option<&str>, show_hidden: bool, all: bool) -> Result<Vec<FileEntry>> {
    let dir = normalize_input_path(input);
    let shown_dir = path_string(&dir);
    let rd = fs::read_dir(&dir)
        .map_err(|_| Error::forbidden(format!("Cannot read folder: {shown_dir}")))?;
    let mut files: Vec<FileEntry> = rd
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_ok_and(|t| !t.is_dir()))
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let env = is_env_file(&name);
            let shown = env || (all && (show_hidden || !platform::is_hidden(&name, Some(&e))));
            shown.then(|| FileEntry {
                path: path_string(&dir.join(&name)),
                name,
                env,
            })
        })
        .collect();
    files.sort_by(|a, b| {
        b.env
            .cmp(&a.env)
            .then_with(|| locale_cmp_base(&a.name, &b.name))
    });
    files.truncate(MAX_DIRS);
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_env_files_first() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        for f in [
            ".env.local",
            "notes.txt",
            ".env",
            "prod.env",
            ".bashrc",
            ".envrc",
        ] {
            fs::write(root.join(f), "").unwrap();
        }
        fs::create_dir(root.join("sub.env")).unwrap();
        let names = |all, hidden| -> Vec<String> {
            list_files(Some(&path_string(root)), hidden, all)
                .unwrap()
                .into_iter()
                .map(|f| f.name)
                .collect()
        };
        assert_eq!(names(false, false), [".env", ".env.local", "prod.env"]);
        assert_eq!(
            names(true, false),
            [".env", ".env.local", "prod.env", "notes.txt"]
        );
        assert_eq!(names(true, true).len(), 6);
        assert!(!is_env_file(".envrc") && is_env_file("Local.ENV"));
    }

    #[test]
    fn lists_sub_folders() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        for d in ["b", "A", ".hidden", "proj", "sln"] {
            fs::create_dir(root.join(d)).unwrap();
        }
        fs::write(root.join("proj/package.json"), "{}").unwrap();
        fs::write(root.join("sln/App.SLN"), "").unwrap();
        fs::write(root.join("file.txt"), "").unwrap();
        let l = list_dirs(Some(&path_string(root)), false).unwrap();
        let names: Vec<_> = l
            .dirs
            .iter()
            .map(|d| (d.name.as_str(), d.project))
            .collect();
        assert_eq!(
            names,
            [("A", false), ("b", false), ("proj", true), ("sln", true)]
        );
        assert_eq!(
            l.parent.as_deref(),
            root.parent().map(path_string).as_deref()
        );
        let l = list_dirs(Some(&path_string(root)), true).unwrap();
        assert_eq!(l.dirs[0].name, ".hidden");

        let e = list_dirs(Some(&path_string(&root.join("file.txt"))), false).unwrap_err();
        assert!(e.message.starts_with("Not a folder: "));
        let e = list_dirs(Some(&path_string(&root.join("nope"))), false).unwrap_err();
        assert!(e.message.starts_with("Folder not found: "));
    }

    #[cfg(unix)]
    #[test]
    fn root_has_no_parent() {
        assert_eq!(list_dirs(Some("/"), false).unwrap().parent, None);
    }
}
