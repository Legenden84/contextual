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

/// Resolves an explicitly chosen attachment on the machine running the UI, not the
/// workspace host. Windows paths delivered to a WSLg app need WSL's mount translation.
/// Native paths (including non-UTF-8 Unix paths) are preserved.
pub async fn local_attachment_path(path: PathBuf) -> crate::Result<PathBuf> {
    let in_wsl = cfg!(target_os = "linux")
        && (std::env::var_os("WSL_DISTRO_NAME").is_some()
            || std::env::var_os("WSL_INTEROP").is_some());
    resolve_attachment_path(path, in_wsl, &crate::process::SystemRunner::default()).await
}

async fn resolve_attachment_path(
    path: PathBuf,
    in_wsl: bool,
    runner: &dyn crate::process::CommandRunner,
) -> crate::Result<PathBuf> {
    let Some(text) = path.to_str() else {
        return Ok(path);
    };
    let bytes = text.as_bytes();
    let drive_absolute = bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/');
    let unc = text.starts_with(r"\\") || text.starts_with("//");
    if !in_wsl || !(drive_absolute || unc) {
        return Ok(path);
    }

    // Do not assume /mnt/c: wslpath respects the distro's configured drive mounts.
    // The path is one argument, never shell input.
    let output = runner
        .run(
            crate::process::Cmd::new("wslpath", ["-u", text])
                .timeout(std::time::Duration::from_secs(5)),
        )
        .await;
    if !output.ok {
        return Err(crate::Error::invalid(format!(
            "Could not access the Windows attachment from WSL: {}. \
             Use Attach images… and select the file from its mounted Windows drive.",
            output.stderr.trim()
        )));
    }
    // Remove the command's line ending, not whitespace belonging to the filename.
    let translated = output.stdout.strip_suffix('\n').unwrap_or(&output.stdout);
    let translated = translated.strip_suffix('\r').unwrap_or(translated);
    if !translated.starts_with('/') || translated.starts_with("//") {
        return Err(crate::Error::invalid(
            "The Windows attachment has no local WSL path. \
             Use Attach images… and select a file accessible from this distro.",
        ));
    }
    Ok(PathBuf::from(translated))
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

    struct AttachmentRunner {
        output: crate::process::RunOutput,
        calls: std::sync::Mutex<Vec<crate::process::Cmd>>,
    }

    impl AttachmentRunner {
        fn new(output: crate::process::RunOutput) -> Self {
            Self {
                output,
                calls: std::sync::Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait::async_trait]
    impl crate::process::CommandRunner for AttachmentRunner {
        async fn run(&self, cmd: crate::process::Cmd) -> crate::process::RunOutput {
            self.calls.lock().unwrap().push(cmd);
            self.output.clone()
        }
    }

    #[tokio::test]
    async fn attachment_paths_are_unchanged_without_translation() {
        let runner = AttachmentRunner::new(crate::process::RunOutput::default());
        for (input, in_wsl) in [
            (r"C:\Users\me\picture.png", false),
            (r"\\server\share\picture.png", false),
            ("/Users/me/picture.png", false),
            ("/mnt/c/Users/me/picture.png", true),
            ("/home/me/picture.png", true),
            ("relative.png", true),
        ] {
            let path = PathBuf::from(input);
            assert_eq!(
                resolve_attachment_path(path.clone(), in_wsl, &runner)
                    .await
                    .unwrap(),
                path
            );
        }
        assert!(runner.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn windows_attachments_use_wslpath_as_one_argument() {
        let runner = AttachmentRunner::new(crate::process::RunOutput::success(
            "/windows/c/Users/me/雪 & photo.png\n",
        ));
        for input in [
            r"C:\Users\me\雪 & photo.png",
            "C:/Users/me/雪 & photo.png",
            r"\\wsl.localhost\Ubuntu\home\me\photo.png",
        ] {
            assert_eq!(
                resolve_attachment_path(PathBuf::from(input), true, &runner)
                    .await
                    .unwrap(),
                PathBuf::from("/windows/c/Users/me/雪 & photo.png")
            );
            let calls = runner.calls.lock().unwrap();
            let call = calls.last().unwrap();
            assert_eq!(call.program, "wslpath");
            assert_eq!(call.args, vec!["-u", input]);
            assert_eq!(call.timeout, std::time::Duration::from_secs(5));
        }
    }

    #[tokio::test]
    async fn attachment_translation_reports_failures() {
        for output in [
            crate::process::RunOutput::failure(1, "drive is not mounted"),
            crate::process::RunOutput::success(""),
            crate::process::RunOutput::success("relative.png\n"),
            crate::process::RunOutput::success("//server/share/photo.png\n"),
        ] {
            let runner = AttachmentRunner::new(output);
            let error = resolve_attachment_path(
                PathBuf::from(r"C:\Users\me\photo.png"),
                true,
                &runner,
            )
            .await
            .unwrap_err();
            assert!(error.message.contains("Attach images"));
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn non_utf8_attachment_paths_are_preserved() {
        use std::os::unix::ffi::OsStringExt as _;

        let path = PathBuf::from(std::ffi::OsString::from_vec(b"/tmp/\xff.png".to_vec()));
        let runner = AttachmentRunner::new(crate::process::RunOutput::default());
        assert_eq!(
            resolve_attachment_path(path.clone(), true, &runner)
                .await
                .unwrap(),
            path
        );
        assert!(runner.calls.lock().unwrap().is_empty());
    }

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
