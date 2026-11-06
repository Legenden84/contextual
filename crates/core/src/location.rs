//! Where a project lives, and turning what the user typed into a location + native path
//! (plan §6.6). Replaces the WSL-side translation in the old `fs/paths.ts` for the Windows app.
//!
//! Everything here is pure string logic so it is tested on every OS; [`detect_for_this_os`]
//! picks the rules for the OS the app runs on.

use serde::{Deserialize, Serialize};

use crate::workspace::paths;

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

impl ProjectRoot {
    /// Key for per-project app data (saved chats): the path for local projects,
    /// `wsl://<distro><path>` for WSL ones.
    pub fn key(&self) -> String {
        match &self.location {
            Location::Local => self.path.clone(),
            Location::Wsl { distro } => format!("wsl://{distro}{}", self.path),
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

/// `\\wsl$\Ubuntu\home\me` / `\\wsl.localhost\Ubuntu\home\me`, also with forward slashes
/// → ("Ubuntu", "/home/me"). Trailing separators are dropped.
pub fn parse_wsl_unc(p: &str) -> Option<(String, String)> {
    let (distro, path) = paths::parse_wsl_unc(&p.replace('/', "\\"))?;
    let trimmed = path.trim_end_matches('/');
    Some((
        distro,
        if trimmed.is_empty() {
            "/".into()
        } else {
            trimmed.to_string()
        },
    ))
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
        if let Some(win) = paths::mnt_to_drive(&path) {
            return Detected::Root(ProjectRoot {
                location: Location::Local,
                path: win,
            });
        }
        return Detected::Root(ProjectRoot {
            location: Location::Wsl { distro },
            path,
        });
    }
    if p.starts_with('/') {
        if let Some(win) = paths::mnt_to_drive(p) {
            return Detected::Root(ProjectRoot {
                location: Location::Local,
                path: win,
            });
        }
        return match default_distro {
            Some(d) => Detected::Root(ProjectRoot {
                location: Location::Wsl {
                    distro: d.to_string(),
                },
                path: p.to_string(),
            }),
            None => Detected::NeedsWsl(p.to_string()),
        };
    }
    Detected::Root(ProjectRoot {
        location: Location::Local,
        path: p.to_string(),
    })
}

/// Rules for macOS (and the Linux agent): native paths only.
pub fn detect_posix(input: &str) -> Detected {
    Detected::Root(ProjectRoot {
        location: Location::Local,
        path: clean(input).to_string(),
    })
}

pub fn detect_for_this_os(input: &str, default_distro: Option<&str>) -> Detected {
    if cfg!(windows) {
        detect_windows(input, default_distro)
    } else {
        detect_posix(input)
    }
}

/// The folder in a `contextual://open?path=<percent-encoded path>` deep link.
pub fn parse_open_url(url: &str) -> Option<String> {
    let rest = url
        .strip_prefix("contextual://open")?
        .trim_start_matches('/');
    let query = rest.strip_prefix('?')?;
    let value = query.split('&').find_map(|kv| kv.strip_prefix("path="))?;
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok()?;
                out.push(u8::from_str_radix(hex, 16).ok()?);
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    let path = String::from_utf8(out).ok()?;
    (!path.trim().is_empty()).then_some(path)
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
        assert_eq!(
            parse_wsl_unc(r"\\wsl.localhost\Ubuntu\home\me\x"),
            Some(("Ubuntu".into(), "/home/me/x".into()))
        );
        assert_eq!(
            parse_wsl_unc(r"\\WSL$\Debian"),
            Some(("Debian".into(), "/".into()))
        );
        assert_eq!(
            parse_wsl_unc("//wsl$/Ubuntu/home/"),
            Some(("Ubuntu".into(), "/home".into()))
        );
        assert_eq!(parse_wsl_unc(r"\\server\share\x"), None);
        assert_eq!(parse_wsl_unc(r"\\wsl$\"), None);
    }

    #[test]
    fn windows_detection() {
        let wsl = |d: &str| Location::Wsl { distro: d.into() };
        assert_eq!(
            root(detect_windows(r"C:\src\x", Some("Ubuntu"))),
            ProjectRoot {
                location: Location::Local,
                path: r"C:\src\x".into()
            }
        );
        assert_eq!(
            root(detect_windows(
                r#""\\wsl.localhost\Ubuntu\home\me\x""#,
                None
            )),
            ProjectRoot {
                location: wsl("Ubuntu"),
                path: "/home/me/x".into()
            }
        );
        assert_eq!(
            root(detect_windows("/home/me/x", Some("Ubuntu"))),
            ProjectRoot {
                location: wsl("Ubuntu"),
                path: "/home/me/x".into()
            }
        );
        assert_eq!(
            detect_windows("/home/me/x", None),
            Detected::NeedsWsl("/home/me/x".into())
        );
        assert_eq!(
            root(detect_windows("/mnt/c/src/x", None)),
            ProjectRoot {
                location: Location::Local,
                path: r"C:\src\x".into()
            }
        );
        assert_eq!(
            root(detect_windows(r"\\wsl$\Ubuntu\mnt\c\src", None)),
            ProjectRoot {
                location: Location::Local,
                path: r"C:\src".into()
            }
        );
    }

    #[test]
    fn labels() {
        assert_eq!(Location::Local.label(), "Local");
        assert_eq!(
            Location::Wsl {
                distro: "Ubuntu".into()
            }
            .label(),
            "WSL: Ubuntu"
        );
    }

    #[test]
    fn deep_links() {
        assert_eq!(
            parse_open_url("contextual://open?path=%2Fhome%2Fme%2Fx").as_deref(),
            Some("/home/me/x")
        );
        assert_eq!(
            parse_open_url("contextual://open/?path=C%3A%5Csrc%5Capp&x=1").as_deref(),
            Some(r"C:\src\app")
        );
        assert_eq!(
            parse_open_url("contextual://open?path=a+b").as_deref(),
            Some("a b")
        );
        assert_eq!(parse_open_url("contextual://open?path=").as_deref(), None);
        assert_eq!(parse_open_url("https://open?path=x"), None);
        assert_eq!(parse_open_url("contextual://open?path=%ZZ"), None);
    }
}
