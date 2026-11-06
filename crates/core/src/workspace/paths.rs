//! Path handling for the workspace host (port of `src/server/fs/paths.ts`).
//!
//! - [`normalize_input_path`] turns what the user typed into an absolute path. On Linux (the
//!   WSL agent and Linux builds) Windows-style input is translated as today:
//!   `C:\src\x` → `/mnt/c/src/x`, `\\wsl$\Ubuntu\home` / `\\wsl.localhost\Ubuntu\home` → `/home`.
//!   On Windows and macOS only native paths (and `~`) are accepted; the app's location detection
//!   (plan §6.6) uses the pure helpers [`parse_wsl_unc`], [`drive_to_mnt`] and [`mnt_to_drive`],
//!   which are compiled and tested on every OS.
//! - [`resolve_inside`] / [`resolve_inside_real`] keep project paths inside the project root;
//!   the comparison is case-insensitive on Windows and macOS (plan §7).

use std::path::{Component, MAIN_SEPARATOR, Path, PathBuf};

use crate::error::{Error, Result};
use crate::platform;

use super::condense::js_trim;

/// Lossy `String` form of a path, as sent over the wire.
pub fn path_string(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

/// JavaScript's `.` (any char except line terminators), for porting `(.*)$` captures.
fn no_line_terminators(s: &str) -> bool {
    !s.chars()
        .any(|c| matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}'))
}

/// `\\wsl$\Distro\rest` or `\\wsl.localhost\Distro\rest` (case-insensitive) → `("Distro", "/rest")`.
pub fn parse_wsl_unc(p: &str) -> Option<(String, String)> {
    let lower = p.to_ascii_lowercase();
    let after = ["\\\\wsl$\\", "\\\\wsl.localhost\\"]
        .iter()
        .find(|pre| lower.starts_with(*pre))
        .map(|pre| &p[pre.len()..])?;
    let (distro, rest) = match after.find('\\') {
        Some(i) => (&after[..i], Some(&after[i + 1..])),
        None => (after, None),
    };
    if distro.is_empty() || rest.is_some_and(|r| !no_line_terminators(r)) {
        return None;
    }
    Some((
        distro.to_string(),
        format!("/{}", rest.unwrap_or("").replace('\\', "/")),
    ))
}

/// `C:\src\x` or `c:/src/x` (or bare `C:`) → `/mnt/c/src/x` (not yet normalised).
pub fn drive_to_mnt(p: &str) -> Option<String> {
    let b = p.as_bytes();
    if b.len() < 2 || !b[0].is_ascii_alphabetic() || b[1] != b':' {
        return None;
    }
    let rest = match b.get(2) {
        None => "",
        Some(b'\\' | b'/') => &p[3..],
        Some(_) => return None,
    };
    if !no_line_terminators(rest) {
        return None;
    }
    Some(format!(
        "/mnt/{}/{}",
        (b[0] as char).to_ascii_lowercase(),
        rest.replace('\\', "/")
    ))
}

/// `/mnt/c/src/x` → `C:\src\x` (WSL's view of a Windows drive back to the native path).
pub fn mnt_to_drive(p: &str) -> Option<String> {
    let rest = p.strip_prefix("/mnt/")?;
    let mut parts = rest.splitn(2, '/');
    let drive = parts.next()?;
    if drive.len() != 1 || !drive.as_bytes()[0].is_ascii_lowercase() {
        return None;
    }
    let tail: Vec<&str> = parts
        .next()
        .unwrap_or("")
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();
    Some(format!(
        "{}:\\{}",
        drive.to_ascii_uppercase(),
        tail.join("\\")
    ))
}

/// Today's Linux translation of Windows-style input (WSL UNC paths first, then drive paths).
pub fn windows_to_posix(p: &str) -> Option<String> {
    if let Some((_, posix)) = parse_wsl_unc(p) {
        return Some(posix);
    }
    drive_to_mnt(p)
}

/// Lexically normalises an absolute path like Node's `path.resolve`: drops `.`, applies `..`
/// (never above the root), collapses repeated separators and trailing slashes.
pub fn normalize_lexically(p: &Path) -> PathBuf {
    let mut out: Vec<Component<'_>> = Vec::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => match out.last() {
                Some(Component::Normal(_)) => {
                    out.pop();
                }
                Some(Component::RootDir | Component::Prefix(_)) => {}
                _ => out.push(c),
            },
            _ => out.push(c),
        }
    }
    let mut buf = PathBuf::new();
    for c in out {
        buf.push(c.as_os_str());
    }
    if buf.as_os_str().is_empty() {
        buf.push(".");
    }
    buf
}

/// `path.resolve(base, p)`.
pub fn resolve(base: &Path, p: &Path) -> PathBuf {
    let joined = if p.is_absolute() {
        p.to_path_buf()
    } else {
        base.join(p)
    };
    #[cfg(windows)]
    let joined = std::path::absolute(&joined).unwrap_or(joined);
    normalize_lexically(&joined)
}

fn cwd() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| platform::home_dir())
}

/// Turns what the user typed into an absolute path on this machine.
pub fn normalize_input_path(input: Option<&str>) -> PathBuf {
    let trimmed = js_trim(input.unwrap_or(""));
    let unquoted = trimmed.strip_prefix(['"', '\'']).unwrap_or(trimmed);
    let p = unquoted.strip_suffix(['"', '\'']).unwrap_or(unquoted);
    if p.is_empty() || p == "~" {
        return platform::home_dir();
    }
    let home_rest = p.strip_prefix("~/");
    #[cfg(windows)]
    let home_rest = home_rest.or_else(|| p.strip_prefix("~\\"));
    if let Some(rest) = home_rest {
        return resolve(&platform::home_dir(), Path::new(rest));
    }

    #[cfg(target_os = "linux")]
    if let Some(posix) = windows_to_posix(p) {
        return resolve(&cwd(), Path::new(&posix));
    }
    resolve(&cwd(), Path::new(p))
}

/// Whether `rel` is rooted (`/x`; on Windows also `\x`, `C:x`, `C:\x`, UNC).
fn is_rooted(rel: &str) -> bool {
    let p = Path::new(rel);
    p.has_root() || p.is_absolute() || matches!(p.components().next(), Some(Component::Prefix(_)))
}

/// Pure containment check on normalised path strings: `candidate` is `root` or below it.
pub fn is_inside_str(root: &str, candidate: &str, sep: char, case_insensitive: bool) -> bool {
    let (root, candidate) = if case_insensitive {
        (root.to_lowercase(), candidate.to_lowercase())
    } else {
        (root.to_string(), candidate.to_string())
    };
    if candidate == root {
        return true;
    }
    if root.ends_with(sep) {
        candidate.starts_with(&root)
    } else {
        candidate.len() > root.len()
            && candidate.starts_with(&root)
            && candidate[root.len()..].starts_with(sep)
    }
}

/// `candidate` is `root` or inside it, with this OS's separator and case rules.
pub fn is_inside(root: &Path, candidate: &Path) -> bool {
    let fix = |p: &Path| {
        let s = path_string(p);
        if cfg!(windows) {
            s.replace('/', "\\")
        } else {
            s
        }
    };
    is_inside_str(
        &fix(root),
        &fix(candidate),
        MAIN_SEPARATOR,
        platform::CASE_INSENSITIVE_PATHS,
    )
}

/// Validates a project-relative path (`src/a.cs`) and returns its absolute path inside `root`.
pub fn resolve_inside(root: &Path, rel: &str) -> Result<PathBuf> {
    if rel.is_empty() || rel.contains('\0') || is_rooted(rel) {
        return Err(Error::invalid(format!("Invalid project path: {rel}")));
    }
    let abs = resolve(root, Path::new(rel));
    if !is_inside(root, &abs) {
        return Err(Error::forbidden(format!("Path escapes the project: {rel}")));
    }
    Ok(abs)
}

/// Like [`resolve_inside`], but also follows symlinks of the deepest existing ancestor to keep
/// reads and writes inside the project. Fails if the root itself can't be resolved.
pub fn resolve_inside_real(root: &Path, rel: &str) -> Result<PathBuf> {
    let abs = resolve_inside(root, rel)?;
    let real_root = dunce::canonicalize(root)?;
    check_real(&real_root, &abs, rel)?;
    Ok(abs)
}

/// The symlink part of [`resolve_inside_real`]: the deepest existing ancestor of `abs` must
/// resolve inside `real_root` (the canonical project root).
pub fn check_real(real_root: &Path, abs: &Path, rel: &str) -> Result<()> {
    let mut probe = abs;
    loop {
        if let Ok(real) = dunce::canonicalize(probe) {
            if !is_inside(real_root, &real) {
                return Err(Error::forbidden(format!(
                    "Path leaves the project through a symlink: {rel}"
                )));
            }
            return Ok(());
        }
        match probe.parent() {
            Some(parent) if parent != probe => probe = parent,
            _ => return Ok(()),
        }
    }
}

/// Project-relative path with forward slashes (`path.relative(root, abs)`).
pub fn to_rel(root: &Path, abs: &Path) -> String {
    let r: Vec<_> = root.components().collect();
    let a: Vec<_> = abs.components().collect();
    let common = r.iter().zip(&a).take_while(|(x, y)| x == y).count();
    let mut parts: Vec<String> = vec!["..".to_string(); r.len() - common];
    parts.extend(
        a[common..]
            .iter()
            .map(|c| c.as_os_str().to_string_lossy().into_owned()),
    );
    parts.join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wsl_unc_paths() {
        assert_eq!(
            parse_wsl_unc(r"\\wsl$\Ubuntu\home\me"),
            Some(("Ubuntu".into(), "/home/me".into()))
        );
        assert_eq!(
            parse_wsl_unc(r"\\WSL.LOCALHOST\Debian\srv"),
            Some(("Debian".into(), "/srv".into()))
        );
        assert_eq!(
            parse_wsl_unc(r"\\wsl$\Ubuntu"),
            Some(("Ubuntu".into(), "/".into()))
        );
        assert_eq!(
            parse_wsl_unc(r"\\wsl$\Ubuntu\"),
            Some(("Ubuntu".into(), "/".into()))
        );
        assert_eq!(parse_wsl_unc(r"\\wsl$\\x"), None);
        assert_eq!(parse_wsl_unc(r"\\server\share"), None);
        assert_eq!(parse_wsl_unc("/home/me"), None);
    }

    #[test]
    fn drive_paths() {
        assert_eq!(drive_to_mnt(r"C:\src\x").as_deref(), Some("/mnt/c/src/x"));
        assert_eq!(drive_to_mnt("d:/a/b").as_deref(), Some("/mnt/d/a/b"));
        assert_eq!(drive_to_mnt("C:").as_deref(), Some("/mnt/c/"));
        assert_eq!(drive_to_mnt("C:foo"), None);
        assert_eq!(drive_to_mnt("1:/x"), None);
        assert_eq!(mnt_to_drive("/mnt/c/src/x").as_deref(), Some(r"C:\src\x"));
        assert_eq!(mnt_to_drive("/mnt/d").as_deref(), Some(r"D:\"));
        assert_eq!(mnt_to_drive("/mnt/data/x"), None);
        assert_eq!(mnt_to_drive("/home/x"), None);
        assert_eq!(
            windows_to_posix(r"\\wsl.localhost\U\home").as_deref(),
            Some("/home")
        );
        assert_eq!(windows_to_posix(r"E:\").as_deref(), Some("/mnt/e/"));
    }

    #[test]
    fn containment_strings() {
        assert!(is_inside_str("/a/b", "/a/b", '/', false));
        assert!(is_inside_str("/a/b", "/a/b/c", '/', false));
        assert!(!is_inside_str("/a/b", "/a/bc", '/', false));
        assert!(!is_inside_str("/a/b", "/a", '/', false));
        assert!(is_inside_str("/", "/etc", '/', false));
        assert!(!is_inside_str("/a/B", "/a/b/c", '/', false));
        // Windows / macOS: case-insensitive.
        assert!(is_inside_str(
            "/Users/Me/Proj",
            "/users/me/proj/src",
            '/',
            true
        ));
        assert!(is_inside_str(r"C:\Src\X", r"c:\src\x\a.cs", '\\', true));
        assert!(!is_inside_str(r"C:\Src\X", r"C:\Src\Xy\a.cs", '\\', true));
        assert!(is_inside_str(r"C:\", r"C:\a", '\\', true));
    }

    #[test]
    fn lexical_normalisation() {
        let n = |s: &str| path_string(&normalize_lexically(Path::new(s)));
        if cfg!(unix) {
            assert_eq!(n("/a/./b/../c/"), "/a/c");
            assert_eq!(n("//a//b"), "/a/b");
            assert_eq!(n("/../x"), "/x");
            assert_eq!(n("/"), "/");
        }
    }

    #[cfg(unix)]
    #[test]
    fn resolve_inside_rules() {
        let root = Path::new("/p/root");
        assert_eq!(
            resolve_inside(root, "src/a.cs").unwrap(),
            Path::new("/p/root/src/a.cs")
        );
        assert_eq!(
            resolve_inside(root, "a/../b").unwrap(),
            Path::new("/p/root/b")
        );
        assert_eq!(resolve_inside(root, ".").unwrap(), Path::new("/p/root"));
        let e = resolve_inside(root, "../x").unwrap_err();
        assert_eq!(e.message, "Path escapes the project: ../x");
        assert_eq!(e.code, crate::error::ErrorCode::Forbidden);
        let e = resolve_inside(root, "/etc/passwd").unwrap_err();
        assert_eq!(e.message, "Invalid project path: /etc/passwd");
        assert!(resolve_inside(root, "").is_err());
        assert!(resolve_inside(root, "a\0b").is_err());
        assert!(resolve_inside(Path::new("/"), "etc").is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn normalize_input() {
        let home = platform::home_dir();
        assert_eq!(normalize_input_path(None), home);
        assert_eq!(normalize_input_path(Some("  ~ ")), home);
        assert_eq!(normalize_input_path(Some("~/x/../y")), home.join("y"));
        assert_eq!(
            normalize_input_path(Some("\"/tmp/a b/\"")),
            Path::new("/tmp/a b")
        );
        assert_eq!(normalize_input_path(Some("'/x'")), Path::new("/x"));
        assert_eq!(normalize_input_path(Some("rel")), cwd().join("rel"));
        if cfg!(target_os = "linux") {
            assert_eq!(
                normalize_input_path(Some(r"C:\src\x")),
                Path::new("/mnt/c/src/x")
            );
            assert_eq!(normalize_input_path(Some("C:")), Path::new("/mnt/c"));
            assert_eq!(
                normalize_input_path(Some(r"\\wsl$\Ubuntu\home\me\")),
                Path::new("/home/me")
            );
        }
    }

    #[test]
    fn relative_paths() {
        if cfg!(unix) {
            assert_eq!(to_rel(Path::new("/r"), Path::new("/r/a/b")), "a/b");
            assert_eq!(to_rel(Path::new("/r"), Path::new("/r")), "");
            assert_eq!(to_rel(Path::new("/r/x"), Path::new("/r/y")), "../y");
        }
    }

    #[cfg(unix)]
    #[test]
    fn symlink_escape_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("proj");
        let outside = tmp.path().join("outside");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, root.join("out")).unwrap();
        std::os::unix::fs::symlink(root.join("src"), root.join("inner")).unwrap();
        assert!(resolve_inside_real(&root, "src/new/file.txt").is_ok());
        assert!(resolve_inside_real(&root, "inner/file.txt").is_ok());
        let e = resolve_inside_real(&root, "out/new/file.txt").unwrap_err();
        assert_eq!(
            e.message,
            "Path leaves the project through a symlink: out/new/file.txt"
        );
        assert!(resolve_inside_real(&tmp.path().join("missing"), "a").is_err());
    }
}
