//! Recently opened projects and the project to reopen on start (`state.json` in the data dir;
//! replaces the old app's localStorage and sessionStorage).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::location::Location;
use crate::{Error, Result};

/// How many recent projects are kept (as before).
pub const MAX_RECENTS: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentProject {
    pub name: String,
    pub path: String,
    #[serde(default = "local")]
    pub location: Location,
    /// RFC 3339 timestamp.
    pub opened_at: String,
}

fn local() -> Location {
    Location::Local
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AppState {
    pub recents: Vec<RecentProject>,
    /// Reopened on start, like the old app's reload behaviour.
    pub last_open: Option<RecentProject>,
}

impl AppState {
    /// Puts `project` first (dropping an older entry for the same place) and remembers it as open.
    pub fn opened(&mut self, project: RecentProject) {
        self.recents
            .retain(|r| !(r.path == project.path && r.location == project.location));
        self.recents.insert(0, project.clone());
        self.recents.truncate(MAX_RECENTS);
        self.last_open = Some(project);
    }

    /// Removes only the history entry matching both path and location.
    /// The remembered open project is unchanged.
    pub fn remove_recent(&mut self, path: &str, location: &Location) {
        self.recents
            .retain(|r| !(r.path == path && &r.location == location));
    }
}

#[derive(Debug, Clone)]
pub struct StateStore {
    path: PathBuf,
}

impl StateStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// `<data dir>/state.json`.
    pub fn open_default() -> Result<Self> {
        directories::ProjectDirs::from("", "", "contextual")
            .map(|d| Self::new(d.data_dir().join("state.json")))
            .ok_or_else(|| Error::internal("No home directory."))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The saved state; a missing or unreadable file means a fresh start.
    pub fn load(&self) -> AppState {
        std::fs::read_to_string(&self.path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, state: &AppState) -> Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = self
            .path
            .with_extension(format!("json.{}.tmp", std::process::id()));
        std::fs::write(&tmp, serde_json::to_vec_pretty(state)?)?;
        std::fs::rename(&tmp, &self.path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(path: &str) -> RecentProject {
        RecentProject {
            name: path.into(),
            path: path.into(),
            location: Location::Local,
            opened_at: "t".into(),
        }
    }

    #[test]
    fn recents_dedupe_and_cap() {
        let mut s = AppState::default();
        for i in 0..10 {
            s.opened(p(&format!("/p{i}")));
        }
        s.opened(p("/p5"));
        assert_eq!(s.recents.len(), MAX_RECENTS);
        assert_eq!(s.recents[0].path, "/p5");
        assert_eq!(s.recents.iter().filter(|r| r.path == "/p5").count(), 1);
        assert_eq!(s.last_open.as_ref().unwrap().path, "/p5");
    }

    #[test]
    fn remove_recent_preserves_other_entries_and_order() {
        let mut s = AppState::default();
        s.opened(p("/first"));
        s.opened(p("/middle"));
        s.opened(p("/last"));

        s.remove_recent("/middle", &Location::Local);

        assert_eq!(s.recents, vec![p("/last"), p("/first")]);
    }

    #[test]
    fn remove_recent_matches_location_as_well_as_path() {
        let ubuntu = RecentProject {
            location: Location::Wsl {
                distro: "Ubuntu".into(),
            },
            ..p("/project")
        };
        let debian = RecentProject {
            location: Location::Wsl {
                distro: "Debian".into(),
            },
            ..p("/project")
        };
        let mut s = AppState::default();
        s.opened(p("/project"));
        s.opened(ubuntu.clone());
        s.opened(debian.clone());

        s.remove_recent("/project", &ubuntu.location);

        assert_eq!(s.recents, vec![debian, p("/project")]);
    }

    #[test]
    fn remove_recent_ignores_missing_entries() {
        let mut s = AppState::default();
        s.opened(p("/project"));
        let before = s.clone();

        s.remove_recent("/missing", &Location::Local);

        assert_eq!(s, before);
    }

    #[test]
    fn remove_recent_persists_empty_history_without_changing_last_open() {
        let dir = tempfile::tempdir().unwrap();
        let store = StateStore::new(dir.path().join("state.json"));
        let mut s = AppState::default();
        s.opened(p("/project"));
        store.save(&s).unwrap();

        s.remove_recent("/project", &Location::Local);
        store.save(&s).unwrap();

        assert_eq!(
            store.load(),
            AppState {
                recents: Vec::new(),
                last_open: Some(p("/project")),
            }
        );
    }

    #[test]
    fn round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let store = StateStore::new(dir.path().join("state.json"));
        assert_eq!(store.load(), AppState::default());
        let mut s = AppState::default();
        s.opened(p("/x"));
        store.save(&s).unwrap();
        assert_eq!(store.load(), s);
    }
}
