//! Saved conversations per project, outside the project folder (port of `threads.ts`):
//! `<dir>/<project hash>/<thread id>.json` plus a small `index.json`, last 30 per project.
//! The thread body is opaque (`serde_json::Value`); the app owns its shape.
//!
//! The project hash is the first 16 hex digits of SHA-1 over the normalised root path, exactly
//! as before, so threads from `~/.cache/contextual/threads/` can be imported by copying.

use crate::workspace::paths::{normalize_input_path, path_string};
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// Threads kept per project; older ones are deleted.
pub const MAX_THREADS: usize = 30;
/// Longest title kept.
const MAX_TITLE: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadSummary {
    pub id: String,
    pub title: String,
    /// ISO 8601 (`2026-11-06T10:00:00.000Z`); newest first.
    pub updated_at: String,
    pub turns: u64,
}

/// The thread store. `dir` is the `threads` folder (one sub-folder per project).
#[derive(Debug, Clone)]
pub struct ThreadStore {
    dir: PathBuf,
}

/// `<data dir>/threads` for this OS (e.g. `~/.local/share/contextual/threads`,
/// `%APPDATA%\contextual\data\threads`, `~/Library/Application Support/contextual/threads`).
pub fn default_threads_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "contextual").map(|d| d.data_dir().join("threads"))
}

/// Where the old Node server kept threads (`$XDG_CACHE_HOME` or `~/.cache`), for import.
pub fn legacy_threads_dir() -> Option<PathBuf> {
    let cache = std::env::var_os("XDG_CACHE_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| directories::BaseDirs::new().map(|b| b.home_dir().join(".cache")))?;
    Some(cache.join("contextual").join("threads"))
}

/// First 16 hex digits of SHA-1 of the normalised root (the per-project folder name).
pub fn project_hash(normalized_root: &str) -> String {
    hex::encode(sha1(normalized_root.as_bytes()))[..16].to_string()
}

fn check_id(id: &str) -> Result<()> {
    let ok = !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if ok {
        Ok(())
    } else {
        Err(Error::invalid("Invalid thread id."))
    }
}

/// Current time as JavaScript's `new Date().toISOString()`.
pub fn now_iso() -> String {
    let now = time::OffsetDateTime::now_utc();
    let fmt = time::macros::format_description!(
        "[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:3]Z"
    );
    now.format(&fmt).unwrap_or_default()
}

fn truncate_chars(s: &str, n: usize) -> String {
    crate::workspace::git::js_truncate(s, n).to_string()
}

fn not_found() -> Error {
    Error::not_found("Conversation not found.")
}

impl ThreadStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// The store in the OS data dir.
    pub fn open_default() -> Result<Self> {
        default_threads_dir()
            .map(Self::new)
            .ok_or_else(|| Error::internal("No home directory."))
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Folder of one project's threads. `root` is a local path, or `wsl://<distro><path>` for a
    /// project inside WSL (hashed as-is: it is not a path on this machine).
    pub fn project_dir(&self, root: &str) -> Result<PathBuf> {
        if root.starts_with("wsl://") {
            return Ok(self.dir.join(project_hash(root)));
        }
        Ok(self
            .dir
            .join(project_hash(&path_string(&normalize_input_path(Some(
                root,
            ))))))
    }

    async fn read_index(dir: &Path) -> Vec<ThreadSummary> {
        let Ok(text) = tokio::fs::read_to_string(dir.join("index.json")).await else {
            return Vec::new();
        };
        match serde_json::from_str::<Value>(&text) {
            Ok(Value::Array(items)) => items
                .into_iter()
                .filter_map(|v| serde_json::from_value(v).ok())
                .collect(),
            _ => Vec::new(),
        }
    }

    /// Writes via a temp file so a crash never leaves half a JSON file.
    async fn write_atomic(file: &Path, data: &[u8]) -> Result<()> {
        let mut tmp = file.as_os_str().to_owned();
        tmp.push(format!(".{}.tmp", std::process::id()));
        let tmp = PathBuf::from(tmp);
        tokio::fs::write(&tmp, data).await?;
        tokio::fs::rename(&tmp, file).await?;
        Ok(())
    }

    async fn write_index(dir: &Path, index: &[ThreadSummary]) -> Result<()> {
        Self::write_atomic(&dir.join("index.json"), &serde_json::to_vec(index)?).await
    }

    /// Saved conversations of a project, newest first.
    pub async fn list(&self, root: &str) -> Result<Vec<ThreadSummary>> {
        let mut list = Self::read_index(&self.project_dir(root)?).await;
        list.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        Ok(list)
    }

    pub async fn load(&self, root: &str, id: &str) -> Result<Value> {
        check_id(id)?;
        let file = self.project_dir(root)?.join(format!("{id}.json"));
        let text = tokio::fs::read_to_string(file)
            .await
            .map_err(|_| not_found())?;
        serde_json::from_str(&text).map_err(|_| not_found())
    }

    /// Saves a conversation now (`updatedAt` = now). `title` is cut to 200 characters; `None`
    /// becomes "Conversation" (as the old route did).
    pub async fn save(
        &self,
        root: &str,
        id: &str,
        title: Option<&str>,
        turns: u64,
        thread: &Value,
    ) -> Result<()> {
        let summary = ThreadSummary {
            id: id.to_string(),
            title: title
                .map(|t| truncate_chars(t, MAX_TITLE))
                .unwrap_or_else(|| "Conversation".into()),
            updated_at: now_iso(),
            turns,
        };
        self.save_summary(root, summary, thread).await
    }

    /// Saves with an explicit summary; keeps the newest 30 and deletes the rest.
    pub async fn save_summary(
        &self,
        root: &str,
        summary: ThreadSummary,
        thread: &Value,
    ) -> Result<()> {
        check_id(&summary.id)?;
        let dir = self.project_dir(root)?;
        tokio::fs::create_dir_all(&dir).await?;
        Self::write_atomic(
            &dir.join(format!("{}.json", summary.id)),
            &serde_json::to_vec(thread)?,
        )
        .await?;

        let id = summary.id.clone();
        let mut index = vec![summary];
        index.extend(
            Self::read_index(&dir)
                .await
                .into_iter()
                .filter(|t| t.id != id),
        );
        index.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        for old in index.iter().skip(MAX_THREADS) {
            let _ = tokio::fs::remove_file(dir.join(format!("{}.json", old.id))).await;
        }
        index.truncate(MAX_THREADS);
        Self::write_index(&dir, &index).await
    }

    pub async fn delete(&self, root: &str, id: &str) -> Result<()> {
        check_id(id)?;
        let dir = self.project_dir(root)?;
        let _ = tokio::fs::remove_file(dir.join(format!("{id}.json"))).await;
        let index: Vec<ThreadSummary> = Self::read_index(&dir)
            .await
            .into_iter()
            .filter(|t| t.id != id)
            .collect();
        Self::write_index(&dir, &index).await
    }

    /// Renames a conversation: the index entry and the saved body's `title` field.
    pub async fn rename(&self, root: &str, id: &str, title: &str) -> Result<()> {
        check_id(id)?;
        let clean = truncate_chars(title.trim(), MAX_TITLE);
        if clean.is_empty() {
            return Err(Error::invalid("The name cannot be empty."));
        }
        let dir = self.project_dir(root)?;
        let file = dir.join(format!("{id}.json"));
        let text = tokio::fs::read_to_string(&file)
            .await
            .map_err(|_| not_found())?;
        let mut thread: Value = serde_json::from_str(&text).map_err(|_| not_found())?;
        match thread.as_object_mut() {
            Some(o) => {
                o.insert("title".into(), Value::String(clean.clone()));
            }
            // `{ ...thread, title }` of a non-object is just `{ title }`.
            None => thread = serde_json::json!({ "title": clean }),
        }
        Self::write_atomic(&file, &serde_json::to_vec(&thread)?).await?;
        let index: Vec<ThreadSummary> = Self::read_index(&dir)
            .await
            .into_iter()
            .map(|mut t| {
                if t.id == id {
                    t.title = clean.clone();
                }
                t
            })
            .collect();
        Self::write_index(&dir, &index).await
    }

    /// Deletes every saved conversation of a project; returns how many there were.
    /// Copies one project's chats from the old app's store (`~/.cache/contextual/threads`,
    /// keyed by the project's path as the old server saw it) into this store under `key`.
    /// Does nothing when this store already has chats for `key`. Returns the files copied.
    pub async fn import_legacy(
        &self,
        legacy_dir: &Path,
        legacy_root: &str,
        key: &str,
    ) -> Result<usize> {
        let src = legacy_dir.join(project_hash(legacy_root));
        let dst = self.project_dir(key)?;
        if !tokio::fs::try_exists(src.join("index.json"))
            .await
            .unwrap_or(false)
            || tokio::fs::try_exists(dst.join("index.json"))
                .await
                .unwrap_or(false)
        {
            return Ok(0);
        }
        tokio::fs::create_dir_all(&dst).await?;
        let mut copied = 0;
        let mut entries = tokio::fs::read_dir(&src).await?;
        while let Some(e) = entries.next_entry().await? {
            let name = e.file_name();
            if name.to_string_lossy().ends_with(".json") && e.file_type().await?.is_file() {
                let data = tokio::fs::read(e.path()).await?;
                Self::write_atomic(&dst.join(&name), &data).await?;
                copied += 1;
            }
        }
        Ok(copied)
    }

    pub async fn delete_all(&self, root: &str) -> Result<usize> {
        let dir = self.project_dir(root)?;
        tokio::fs::create_dir_all(&dir).await?;
        let index = Self::read_index(&dir).await;
        for t in &index {
            let _ = tokio::fs::remove_file(dir.join(format!("{}.json", t.id))).await;
        }
        Self::write_atomic(&dir.join("index.json"), b"[]").await?;
        Ok(index.len())
    }
}

/// SHA-1 (FIPS 180-4). Only used for the legacy-compatible folder name, not for security.
fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [
        0x6745_2301,
        0xEFCD_AB89,
        0x98BA_DCFE,
        0x1032_5476,
        0xC3D2_E1F0,
    ];
    let mut msg = data.to_vec();
    let bit_len = (data.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());
    for block in msg.chunks_exact(64) {
        let mut w = [0u32; 80];
        for (i, word) in block.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = h;
        for (i, wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | (!b & d), 0x5A82_7999),
                20..=39 => (b ^ c ^ d, 0x6ED9_EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1B_BCDC),
                _ => (b ^ c ^ d, 0xCA62_C1D6),
            };
            let t = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(*wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = t;
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e]) {
            *x = x.wrapping_add(y);
        }
    }
    let mut out = [0u8; 20];
    for (i, v) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&v.to_be_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha1_vectors() {
        assert_eq!(
            hex::encode(sha1(b"")),
            "da39a3ee5e6b4b0d3255bfef95601890afd80709"
        );
        assert_eq!(
            hex::encode(sha1(b"abc")),
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
        let long = "abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq";
        assert_eq!(
            hex::encode(sha1(long.as_bytes())),
            "84983e441c3bd26ebaae4aa1f95129e5e54670f1"
        );
        assert_eq!(
            hex::encode(sha1(&[b'a'; 1000])),
            "291e9a6c66994949b57ba5e650361e98fc36b1ba"
        );
    }

    #[test]
    fn iso_format() {
        let s = now_iso();
        assert_eq!(s.len(), 24, "{s}");
        assert!(s.ends_with('Z'));
    }
}
