//! Reading project files for context and previews, and saving applied changes (port of
//! `fs/files.ts`).
//!
//! Saving behaves like "Save" in an editor: files are written in place in the working tree (no
//! staging, no commit), keeping each file's line endings, BOM and final newline. Writes happen
//! only from an explicit Save in the UI; the model never calls this.
//!
//! All functions here are synchronous (`std::fs`) and `Send`; hosts run them on a blocking pool.
//!
//! Undo snapshots live outside the project, in `<cache>/contextual/undo/<project hash>/<id>/`
//! (`files/…` copies plus `manifest.json`), with the same SHA-1 project hash and id format as the
//! TypeScript server; the last 20 per project are kept.

use std::fs::{self, OpenOptions, Permissions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use base64::Engine as _;
use contextual_protocol::fs::{
    ApplyChange, ApplyResult, ChangeStatus, FileContent, ReadOptions, UndoResult,
};
use serde::de::{MapAccess, Visitor};
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::condense::{CONDENSE_MIN_BYTES, condense, is_condensable, js_cmp, js_round};
use super::images::{MAX_IMAGE_BYTES, image_mime, is_image};
use super::paths::{
    check_real, normalize_input_path, path_string, resolve_inside, resolve_inside_real, to_rel,
};
use super::scan::mtime_ms;
use crate::error::{Error, Result};
use crate::platform;

/// Largest file sent as context or shown in a preview.
pub const MAX_READ_BYTES: u64 = 2 * 1024 * 1024;
/// Undo snapshots kept per project; older ones are pruned.
pub const MAX_SNAPSHOTS: usize = 20;

const BOM: char = '\u{FEFF}';

// ---------------------------------------------------------------------------------------------
// Text format
// ---------------------------------------------------------------------------------------------

/// How a file stores its text, so saving can write it back the same way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextFormat {
    pub crlf: bool,
    pub bom: bool,
    pub final_newline: bool,
}

/// New files: LF, no BOM, final newline.
pub const NEW_FILE_FORMAT: TextFormat = TextFormat {
    crlf: false,
    bom: false,
    final_newline: true,
};

/// Splits raw file text into LF-normalised content and the format needed to write it back.
/// CRLF wins when at least as many lines end in CRLF as in a lone LF.
pub fn decode(raw: &str) -> (String, TextFormat) {
    let bom = raw.starts_with(BOM);
    let body = if bom { &raw[BOM.len_utf8()..] } else { raw };
    let crlf = body.matches("\r\n").count();
    let lf = body.matches('\n').count();
    let format = TextFormat {
        crlf: crlf > 0 && crlf >= lf - crlf,
        bom,
        final_newline: body.ends_with('\n'),
    };
    (body.replace("\r\n", "\n"), format)
}

/// Writes LF text back in the given format, like an editor that keeps a file's settings.
pub fn encode(text: &str, format: TextFormat) -> String {
    let mut body = text.replace("\r\n", "\n");
    // Match the file's final-newline setting either way, so the last line doesn't show as changed.
    if format.final_newline && !body.is_empty() && !body.ends_with('\n') {
        body.push('\n');
    }
    if !format.final_newline && body.ends_with('\n') {
        body.pop();
    }
    if format.crlf {
        body = body.replace('\n', "\r\n");
    }
    if format.bom {
        format!("{BOM}{body}")
    } else {
        body
    }
}

// ---------------------------------------------------------------------------------------------
// SHA-1 (the TypeScript server's project hash and change detector; not used for security)
// ---------------------------------------------------------------------------------------------

/// Hex SHA-1 of `data` (`createHash('sha1').update(string)` hashes the string's UTF-8 bytes).
pub fn sha1_hex(data: &[u8]) -> String {
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
    for chunk in msg.chunks_exact(64) {
        let mut w = [0u32; 80];
        for (wi, b) in w.iter_mut().zip(chunk.chunks_exact(4)) {
            *wi = u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
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
        for (hi, v) in h.iter_mut().zip([a, b, c, d, e]) {
            *hi = hi.wrapping_add(v);
        }
    }
    hex::encode(h.iter().flat_map(|x| x.to_be_bytes()).collect::<Vec<u8>>())
}

// ---------------------------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------------------------

fn too_large(size: u64) -> String {
    format!("too large ({} KB)", js_round(size as f64 / 1024.0))
}

fn read_one(root: &Path, rel: &str, opts: ReadOptions) -> std::result::Result<FileContent, String> {
    let abs = resolve_inside(root, rel).map_err(|e| e.message)?;
    let real_root = dunce::canonicalize(root).map_err(|_| "not found".to_string())?;
    check_real(&real_root, &abs, rel).map_err(|e| e.message)?;

    let not_found = |_| "not found".to_string();
    let meta = fs::metadata(&abs).map_err(not_found)?;
    if !meta.is_file() {
        return Err("not a file".into());
    }
    let mtime = mtime_ms(&meta);
    let size = meta.len();
    let base = FileContent {
        path: rel.to_string(),
        mtime_ms: mtime,
        ..Default::default()
    };
    if is_image(rel) {
        if size > MAX_IMAGE_BYTES {
            return Err(too_large(size));
        }
        let data =
            base64::engine::general_purpose::STANDARD.encode(fs::read(&abs).map_err(not_found)?);
        return Ok(FileContent {
            image: Some(format!("data:{};base64,{data}", image_mime(rel))),
            ..base
        });
    }
    if size > MAX_READ_BYTES {
        return Err(too_large(size));
    }
    let buf = fs::read(&abs).map_err(not_found)?;
    if buf[..buf.len().min(8000)].contains(&0) {
        return Err("binary".into());
    }
    let (text, _) = decode(&String::from_utf8_lossy(&buf));
    if opts.condense && is_condensable(rel) && size >= CONDENSE_MIN_BYTES {
        return Ok(FileContent {
            content: Some(condense(&text)),
            condensed_from: Some(size),
            ..base
        });
    }
    Ok(FileContent {
        content: Some(text),
        ..base
    })
}

/// Reads project files (text up to 2 MB with LF line endings and no BOM; images up to 20 MB as
/// data URLs). Problems are reported per file in `error`, never as a failed call.
pub fn read_files(root_input: &str, paths: &[String], opts: ReadOptions) -> Vec<FileContent> {
    let root = normalize_input_path(Some(root_input));
    paths
        .iter()
        .map(|rel| {
            read_one(&root, rel, opts).unwrap_or_else(|error| FileContent {
                path: rel.clone(),
                error: Some(error),
                ..Default::default()
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Writing
// ---------------------------------------------------------------------------------------------

/// Windows: editors, indexers and antivirus briefly hold files open; retry those errors.
#[cfg(windows)]
fn retry_sharing<T>(mut f: impl FnMut() -> io::Result<T>) -> io::Result<T> {
    const ACCESS_DENIED: i32 = 5;
    const SHARING_VIOLATION: i32 = 32;
    const LOCK_VIOLATION: i32 = 33;
    let mut attempt = 0;
    loop {
        match f() {
            Err(e)
                if attempt < 10
                    && matches!(
                        e.raw_os_error(),
                        Some(ACCESS_DENIED | SHARING_VIOLATION | LOCK_VIOLATION)
                    ) =>
            {
                attempt += 1;
                std::thread::sleep(std::time::Duration::from_millis(50 * attempt));
            }
            r => return r,
        }
    }
}

#[cfg(not(windows))]
fn retry_sharing<T>(mut f: impl FnMut() -> io::Result<T>) -> io::Result<T> {
    f()
}

/// Writes `data` to `target` atomically (temp file in the same folder + rename). Symlinks are
/// written through, like `writeFile`. `perms` defaults to the existing file's permissions; a
/// read-only file is refused as `writeFile` would.
pub fn write_atomic(target: &Path, data: &[u8], perms: Option<Permissions>) -> io::Result<()> {
    let is_link = fs::symlink_metadata(target).is_ok_and(|m| m.file_type().is_symlink());
    let dest = if is_link {
        dunce::canonicalize(target).unwrap_or_else(|_| target.to_path_buf())
    } else {
        target.to_path_buf()
    };
    let existing = fs::metadata(&dest).ok().map(|m| m.permissions());
    if perms.is_none() && existing.as_ref().is_some_and(|p| p.readonly()) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("{} is read-only", path_string(&dest)),
        ));
    }
    let dir = dest
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no parent folder"))?;
    let name = dest
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = dir.join(format!(
        ".{name}.{}.contextual-tmp",
        uuid::Uuid::new_v4().simple()
    ));
    let result = (|| {
        let mut f = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
        drop(f);
        if let Some(p) = perms.or(existing) {
            fs::set_permissions(&tmp, p)?;
        }
        retry_sharing(|| fs::rename(&tmp, &dest))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// `mkdir -p`; returns the first (top-most) folder it had to create, like `mkdir({ recursive })`.
fn create_dirs(dir: &Path) -> io::Result<Option<PathBuf>> {
    let mut first = None;
    let mut probe = Some(dir);
    while let Some(p) = probe {
        if fs::symlink_metadata(p).is_ok() {
            break;
        }
        first = Some(p.to_path_buf());
        probe = p.parent();
    }
    fs::create_dir_all(dir)?;
    Ok(first)
}

fn read_raw(abs: &Path) -> Option<String> {
    fs::read(abs)
        .ok()
        .map(|b| String::from_utf8_lossy(&b).into_owned())
}

/// Written to `manifest.json` in each snapshot (same JSON as the TypeScript server).
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    /// Files that did not exist before (undo deletes them).
    created: Vec<String>,
    /// Files that existed and were copied into the snapshot (undo restores them).
    replaced: Vec<String>,
    /// Folders the save created (undo removes them again if empty), deepest first.
    created_dirs: Vec<String>,
    /// sha1 of what was written (`null` for deletions), so undo won't overwrite later edits.
    written: Written,
}

/// `Record<string, string | null>` keeping insertion order (it decides the order of names in
/// the "edited since the save" message).
#[derive(Debug, Default)]
struct Written(Vec<(String, Option<String>)>);

impl Serialize for Written {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        let mut map = s.serialize_map(Some(self.0.len()))?;
        for (k, v) in &self.0 {
            map.serialize_entry(k, v)?;
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for Written {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Written;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a map of paths to hashes")
            }
            fn visit_map<A: MapAccess<'de>>(
                self,
                mut m: A,
            ) -> std::result::Result<Written, A::Error> {
                let mut out = Vec::new();
                while let Some(entry) = m.next_entry::<String, Option<String>>()? {
                    out.push(entry);
                }
                Ok(Written(out))
            }
        }
        d.deserialize_map(V)
    }
}

/// Default base folder for undo snapshots: `<cache>/contextual/undo`.
pub fn default_undo_dir() -> PathBuf {
    platform::cache_dir().join("undo")
}

/// This project's snapshot folder: `<undo dir>/<first 16 hex of sha1(root)>`.
pub fn project_snapshots_dir(undo_dir: &Path, root: &Path) -> PathBuf {
    undo_dir.join(&sha1_hex(path_string(root).as_bytes())[..16])
}

/// Snapshot id like `new Date().toISOString().replace(/[:.]/g, '-')`.
fn snapshot_id() -> String {
    let fmt = time::macros::format_description!(
        "[year]-[month]-[day]T[hour]-[minute]-[second]-[subsecond digits:3]Z"
    );
    time::OffsetDateTime::now_utc()
        .format(&fmt)
        .unwrap_or_else(|_| uuid::Uuid::new_v4().simple().to_string())
}

fn prune_snapshots(dir: &Path) {
    // Pruning is best effort.
    let Ok(rd) = fs::read_dir(dir) else { return };
    let mut ids: Vec<String> = rd
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    ids.sort_by(|a, b| js_cmp(a, b));
    let excess = ids.len().saturating_sub(MAX_SNAPSHOTS);
    for id in &ids[..excess] {
        let p = dir.join(id);
        let _ = if p.is_dir() {
            fs::remove_dir_all(&p)
        } else {
            fs::remove_file(&p)
        };
    }
}

/// Project path as stored in the manifest and the snapshot (`path.join` normalises it).
fn snapshot_file(snapshot: &Path, root: &Path, abs: &Path) -> PathBuf {
    let mut p = snapshot.join("files");
    for part in to_rel(root, abs).split('/').filter(|s| !s.is_empty()) {
        p.push(part);
    }
    p
}

struct Planned<'a> {
    change: &'a ApplyChange,
    abs: PathBuf,
    current: Option<(String, TextFormat)>,
}

/// Saves the selected changes into the working tree, keeping undo snapshots in
/// [`default_undo_dir`]. See [`apply_changes_in`].
pub fn apply_changes(
    root_input: &str,
    changes: &[ApplyChange],
    backup: bool,
) -> Result<ApplyResult> {
    apply_changes_in(&default_undo_dir(), root_input, changes, backup)
}

/// Saves the selected changes into the working tree. Every file is checked first; if any check
/// fails nothing is written. With `backup`, originals are kept under `undo_dir` for Undo.
pub fn apply_changes_in(
    undo_dir: &Path,
    root_input: &str,
    changes: &[ApplyChange],
    backup: bool,
) -> Result<ApplyResult> {
    let root = normalize_input_path(Some(root_input));
    if changes.is_empty() {
        return Err(Error::invalid("No changes to apply."));
    }

    let mut plan = Vec::with_capacity(changes.len());
    for change in changes {
        let abs = resolve_inside_real(&root, &change.path)?;
        let current = read_raw(&abs).map(|raw| decode(&raw));
        plan.push(Planned {
            change,
            abs,
            current,
        });
    }

    let mut problems = Vec::new();
    for p in &plan {
        let c = p.change;
        if c.status == ChangeStatus::A && p.current.is_some() {
            problems.push(format!("{} already exists", c.path));
        }
        // Not in the TypeScript server, which failed half-way through writing instead.
        if c.status == ChangeStatus::A && p.current.is_none() && p.abs.is_dir() {
            problems.push(format!("{} is a folder", c.path));
        }
        if c.status != ChangeStatus::A && p.current.is_none() {
            problems.push(format!("{} no longer exists", c.path));
        }
        if let (true, Some((text, _)), Some(expected)) =
            (c.status != ChangeStatus::A, &p.current, &c.expected)
        {
            if *text != expected.replace("\r\n", "\n") {
                problems.push(format!("{} changed on disk since it was sent", c.path));
            }
        }
        if c.status != ChangeStatus::D && c.content.is_none() {
            problems.push(format!("{} has no content", c.path));
        }
    }
    if !problems.is_empty() {
        return Err(Error::conflict(format!(
            "Nothing was saved. {}.",
            problems.join("; ")
        )));
    }

    let mut manifest = Manifest::default();
    let mut backup_id = None;
    let mut snapshot = None;
    if backup {
        let dir = project_snapshots_dir(undo_dir, &root);
        let mut id = snapshot_id();
        while dir.join(&id).exists() {
            std::thread::sleep(std::time::Duration::from_millis(1));
            id = snapshot_id();
        }
        let snap = dir.join(&id);
        fs::create_dir_all(snap.join("files"))?;
        for p in plan.iter().filter(|p| p.current.is_some()) {
            let dest = snapshot_file(&snap, &root, &p.abs);
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(&p.abs, &dest)?;
            manifest.replaced.push(p.change.path.clone());
        }
        backup_id = Some(id);
        snapshot = Some(snap);
    }

    let mut result = ApplyResult {
        created: 0,
        overwritten: 0,
        deleted: 0,
        backup_id,
    };
    for p in &plan {
        let c = p.change;
        if c.status == ChangeStatus::D {
            fs::remove_file(&p.abs)?;
            manifest.written.0.push((c.path.clone(), None));
            result.deleted += 1;
            continue;
        }
        let parent = p.abs.parent().unwrap_or(&root);
        if let Some(first) = create_dirs(parent)? {
            // Record every folder from the first one created down to the file's folder.
            let mut d = Some(parent);
            while let Some(dir) = d.filter(|d| d.starts_with(&first)) {
                manifest.created_dirs.push(to_rel(&root, dir));
                d = dir.parent();
            }
        }
        let format = p.current.as_ref().map_or(NEW_FILE_FORMAT, |(_, f)| *f);
        let out = encode(c.content.as_deref().unwrap_or_default(), format);
        write_atomic(&p.abs, out.as_bytes(), None)?;
        manifest
            .written
            .0
            .push((c.path.clone(), Some(sha1_hex(out.as_bytes()))));
        if c.status == ChangeStatus::A {
            manifest.created.push(c.path.clone());
            result.created += 1;
        } else {
            result.overwritten += 1;
        }
    }

    if let Some(snap) = snapshot {
        fs::write(
            snap.join("manifest.json"),
            serde_json::to_string_pretty(&manifest)?,
        )?;
        if let Some(dir) = snap.parent() {
            prune_snapshots(dir);
        }
    }
    Ok(result)
}

/// Restores a snapshot from [`default_undo_dir`]. See [`undo_apply_in`].
pub fn undo_apply(root_input: &str, backup_id: &str) -> Result<UndoResult> {
    undo_apply_in(&default_undo_dir(), root_input, backup_id)
}

/// Restores a snapshot: puts back overwritten and deleted files, removes created files and
/// folders. Refused if any of those files was edited after the save, so later work isn't lost.
pub fn undo_apply_in(undo_dir: &Path, root_input: &str, backup_id: &str) -> Result<UndoResult> {
    let root = normalize_input_path(Some(root_input));
    if backup_id.is_empty()
        || !backup_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err(Error::invalid("Invalid undo id."));
    }
    let snapshot = project_snapshots_dir(undo_dir, &root).join(backup_id);
    let manifest: Manifest = fs::read(snapshot.join("manifest.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .ok_or_else(|| Error::not_found("Nothing to undo: the snapshot is gone."))?;

    let mut edited = Vec::new();
    for (rel, written) in &manifest.written.0 {
        let now = read_raw(&resolve_inside(&root, rel)?).map(|s| sha1_hex(s.as_bytes()));
        if now != *written {
            edited.push(rel.as_str());
        }
    }
    if !edited.is_empty() {
        return Err(Error::conflict(format!(
            "Not undone: edited since the save: {}.",
            edited.join(", ")
        )));
    }

    let mut restored = 0;
    let mut removed = 0;
    for rel in &manifest.replaced {
        let abs = resolve_inside(&root, rel)?;
        if let Some(parent) = abs.parent() {
            fs::create_dir_all(parent)?;
        }
        let src = snapshot_file(&snapshot, &root, &abs);
        let mut data = Vec::new();
        fs::File::open(&src)?.read_to_end(&mut data)?;
        write_atomic(&abs, &data, Some(fs::metadata(&src)?.permissions()))?;
        restored += 1;
    }
    for rel in &manifest.created {
        match fs::remove_file(resolve_inside(&root, rel)?) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e.into()),
            _ => removed += 1,
        }
    }
    // Deepest folders first, so parents are empty by the time they're tried.
    let mut dirs: Vec<&String> = Vec::new();
    for d in &manifest.created_dirs {
        if !dirs.contains(&d) {
            dirs.push(d);
        }
    }
    dirs.sort_by_key(|d| std::cmp::Reverse(d.split('/').count()));
    for rel in dirs {
        // Not empty (something else lives there now): leave it.
        if let Ok(abs) = resolve_inside(&root, rel) {
            let _ = fs::remove_dir(abs);
        }
    }
    let _ = fs::remove_dir_all(&snapshot);
    Ok(UndoResult { restored, removed })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCode;

    fn fmt(crlf: bool, bom: bool, final_newline: bool) -> TextFormat {
        TextFormat {
            crlf,
            bom,
            final_newline,
        }
    }

    #[test]
    fn sha1_vectors() {
        assert_eq!(sha1_hex(b""), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
        assert_eq!(sha1_hex(b"abc"), "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(
            sha1_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "84983e441c3bd26ebaae4aa1f95129e5e54670f1"
        );
        assert_eq!(
            sha1_hex(&[b'a'; 1000]),
            "291e9a6c66994949b57ba5e650361e98fc36b1ba"
        );
    }

    #[test]
    fn decode_formats() {
        assert_eq!(
            decode("a\r\nb\r\n"),
            ("a\nb\n".into(), fmt(true, false, true))
        );
        assert_eq!(
            decode("\u{FEFF}a\nb"),
            ("a\nb".into(), fmt(false, true, false))
        );
        // Mixed: CRLF wins ties.
        assert_eq!(decode("a\r\nb\nc").1, fmt(true, false, false));
        assert_eq!(decode("a\r\nb\nc\n").1, fmt(false, false, true));
        assert_eq!(decode("").1, fmt(false, false, false));
        assert_eq!(decode("x\r").0, "x\r");
    }

    #[test]
    fn encode_round_trips() {
        for raw in [
            "a\r\nb\r\n",
            "\u{FEFF}a\nb",
            "\u{FEFF}x\r\ny",
            "plain\n",
            "",
            "one",
            "\n",
            "\r\n\r\n",
        ] {
            let (text, format) = decode(raw);
            assert_eq!(encode(&text, format), raw, "{raw:?}");
        }
    }

    #[test]
    fn encode_applies_format() {
        assert_eq!(encode("a\nb", fmt(true, true, true)), "\u{FEFF}a\r\nb\r\n");
        assert_eq!(encode("a\r\nb\n\n", fmt(false, false, false)), "a\nb\n");
        assert_eq!(encode("", NEW_FILE_FORMAT), "");
        assert_eq!(encode("x", NEW_FILE_FORMAT), "x\n");
    }

    fn change(
        path: &str,
        status: ChangeStatus,
        content: Option<&str>,
        expected: Option<&str>,
    ) -> ApplyChange {
        ApplyChange {
            path: path.into(),
            status,
            content: content.map(Into::into),
            expected: expected.map(Into::into),
        }
    }

    #[test]
    fn pre_checks_write_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("p");
        let undo = tmp.path().join("undo");
        fs::create_dir_all(root.join("dir")).unwrap();
        fs::write(root.join("a.txt"), "one\r\n").unwrap();
        fs::write(root.join("b.txt"), "two\n").unwrap();
        let r = path_string(&root);
        let changes = [
            change("new/x.txt", ChangeStatus::A, Some("x"), None),
            change("a.txt", ChangeStatus::M, Some("1"), Some("one\n")),
            change("b.txt", ChangeStatus::A, Some("y"), None),
            change("gone.txt", ChangeStatus::M, Some("z"), None),
            change("a.txt", ChangeStatus::M, Some("w"), Some("stale")),
            change("a.txt", ChangeStatus::M, None, None),
            change("dir", ChangeStatus::A, Some("q"), None),
        ];
        let e = apply_changes_in(&undo, &r, &changes, true).unwrap_err();
        assert_eq!(e.code, ErrorCode::Conflict);
        assert_eq!(
            e.message,
            "Nothing was saved. b.txt already exists; gone.txt no longer exists; \
             a.txt changed on disk since it was sent; a.txt has no content; dir is a folder."
        );
        assert!(!root.join("new").exists());
        assert_eq!(fs::read_to_string(root.join("a.txt")).unwrap(), "one\r\n");
        assert!(!undo.exists());

        let e = apply_changes_in(&undo, &r, &[], true).unwrap_err();
        assert_eq!(e.message, "No changes to apply.");
        let e = apply_changes_in(
            &undo,
            &r,
            &[change("../x", ChangeStatus::A, Some("x"), None)],
            true,
        )
        .unwrap_err();
        assert_eq!(e.message, "Path escapes the project: ../x");
    }

    #[test]
    fn apply_then_undo() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("p");
        let undo = tmp.path().join("undo");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("crlf.txt"), "\u{FEFF}a\r\nb").unwrap();
        fs::write(root.join("del.txt"), "bye\n").unwrap();
        let r = path_string(&root);
        let res = apply_changes_in(
            &undo,
            &r,
            &[
                change(
                    "crlf.txt",
                    ChangeStatus::M,
                    Some("a\nB\nc\n"),
                    Some("a\r\nb"),
                ),
                change("x/y/new.txt", ChangeStatus::A, Some("new"), None),
                change("del.txt", ChangeStatus::D, None, Some("bye\n")),
            ],
            true,
        )
        .unwrap();
        assert_eq!((res.created, res.overwritten, res.deleted), (1, 1, 1));
        assert_eq!(
            fs::read_to_string(root.join("crlf.txt")).unwrap(),
            "\u{FEFF}a\r\nB\r\nc"
        );
        assert_eq!(
            fs::read_to_string(root.join("x/y/new.txt")).unwrap(),
            "new\n"
        );
        assert!(!root.join("del.txt").exists());
        let id = res.backup_id.unwrap();
        let snap = project_snapshots_dir(&undo, &root).join(&id);
        let manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(snap.join("manifest.json")).unwrap()).unwrap();
        assert_eq!(manifest["createdDirs"], serde_json::json!(["x/y", "x"]));

        let u = undo_apply_in(&undo, &r, &id).unwrap();
        assert_eq!((u.restored, u.removed), (2, 1));
        assert_eq!(
            fs::read_to_string(root.join("crlf.txt")).unwrap(),
            "\u{FEFF}a\r\nb"
        );
        assert_eq!(fs::read_to_string(root.join("del.txt")).unwrap(), "bye\n");
        assert!(!root.join("x").exists());
        assert!(!snap.exists());
        let e = undo_apply_in(&undo, &r, &id).unwrap_err();
        assert_eq!(e.message, "Nothing to undo: the snapshot is gone.");
        assert_eq!(
            undo_apply_in(&undo, &r, "../x").unwrap_err().message,
            "Invalid undo id."
        );
    }

    #[test]
    fn undo_refused_after_edit() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("p");
        let undo = tmp.path().join("undo");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("a.txt"), "a\n").unwrap();
        let r = path_string(&root);
        let res = apply_changes_in(
            &undo,
            &r,
            &[change("a.txt", ChangeStatus::M, Some("b\n"), None)],
            true,
        )
        .unwrap();
        fs::write(root.join("a.txt"), "edited\n").unwrap();
        let e = undo_apply_in(&undo, &r, res.backup_id.as_deref().unwrap()).unwrap_err();
        assert_eq!(e.code, ErrorCode::Conflict);
        assert_eq!(e.message, "Not undone: edited since the save: a.txt.");
        assert_eq!(fs::read_to_string(root.join("a.txt")).unwrap(), "edited\n");
    }

    #[test]
    fn no_backup_and_pruning() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("p");
        let undo = tmp.path().join("undo");
        fs::create_dir_all(&root).unwrap();
        let r = path_string(&root);
        let res = apply_changes_in(
            &undo,
            &r,
            &[change("n.txt", ChangeStatus::A, Some("n"), None)],
            false,
        )
        .unwrap();
        assert_eq!(res.backup_id, None);
        assert!(!undo.exists());
        let dir = project_snapshots_dir(&undo, &root);
        for i in 0..25 {
            fs::create_dir_all(dir.join(format!("2000-01-01T00-00-00-{i:03}Z"))).unwrap();
        }
        apply_changes_in(
            &undo,
            &r,
            &[change("n.txt", ChangeStatus::M, Some("m"), None)],
            true,
        )
        .unwrap();
        let mut left: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        left.sort();
        assert_eq!(left.len(), MAX_SNAPSHOTS);
        assert_eq!(left[0], "2000-01-01T00-00-00-006Z");
    }

    #[cfg(unix)]
    #[test]
    fn keeps_permissions_and_writes_through_symlinks() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("p");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("run.sh"), "echo\n").unwrap();
        fs::set_permissions(root.join("run.sh"), Permissions::from_mode(0o755)).unwrap();
        std::os::unix::fs::symlink(root.join("run.sh"), root.join("link.sh")).unwrap();
        let r = path_string(&root);
        apply_changes_in(
            &tmp.path().join("u"),
            &r,
            &[change("link.sh", ChangeStatus::M, Some("echo hi"), None)],
            false,
        )
        .unwrap();
        assert!(
            fs::symlink_metadata(root.join("link.sh"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            fs::read_to_string(root.join("run.sh")).unwrap(),
            "echo hi\n"
        );
        assert_eq!(
            fs::metadata(root.join("run.sh"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o755
        );
    }
}
