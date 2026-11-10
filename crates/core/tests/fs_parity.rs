//! Phase 0 parity: replays what the TypeScript server's `src/server/fs/*.ts` returned for the
//! fixture projects (recorded into `tests/fixtures/fs/*.json`; see `tests/fixtures/README.md`)
//! against the Rust port, comparing whole JSON values field by field.
//!
//! Allowances (applied identically to the recording and to the Rust output):
//! - The temp folder the fixtures are copied into is replaced by `<TMP>` in every string.
//! - `mtimeMs` values become `"<mtime>"` (presence is still compared), undo ids become
//!   `"<backupId>"` (`null` is still compared), and the folder browser's `places` are dropped
//!   (they depend on the machine; checked separately against `platform::places()`).
//! - Errors are compared as `{ status, message }`. The Rust error code maps to the old HTTP
//!   status: InvalidInput → 400, Forbidden → 400 (path containment; 403 for "Cannot read
//!   folder"), NotFound → 404, Conflict → 409, anything else → 500.
//! - Known deviation: adding a file where a folder exists. Node passed the checks and failed
//!   while writing (500, `EISDIR`, leaving an empty undo snapshot); Rust refuses it up front
//!   (409, "Nothing was saved. src is a folder.") and keeps no snapshot.
//!
//! Unix only: the fixtures need symlinks, which Windows CI can't create unprivileged.
#![cfg(unix)]

use std::fs;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use contextual_core::error::{Error, ErrorCode};
use contextual_core::platform;
use contextual_core::workspace::{browse, files, scan};
use contextual_protocol::fs::{ApplyChange, ReadOptions, ScanOptions};
use pretty_assertions::assert_eq;
use serde_json::{Map, Value, json};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn load(name: &str) -> Value {
    let path = fixtures().join("fs").join(name);
    serde_json::from_slice(&fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display())))
        .unwrap()
}

fn copy_dir(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).unwrap();
    for e in fs::read_dir(src).unwrap() {
        let e = e.unwrap();
        let to = dst.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &to);
        } else {
            fs::copy(e.path(), &to).unwrap();
        }
    }
}

/// Mirrors `materialize` in the recorder (`record-fs.mjs`, see `fixtures/README.md`).
fn materialize(tmp: &Path) {
    use std::os::unix::fs::symlink;
    copy_dir(&fixtures().join("projects/basic"), &tmp.join("basic"));
    copy_dir(
        &fixtures().join("projects/skill-root"),
        &tmp.join("skill-root"),
    );
    let b = tmp.join("basic");
    fs::create_dir_all(b.join(".git/refs/heads")).unwrap();
    fs::write(b.join(".git/HEAD"), "ref: refs/heads/feature/parity\n").unwrap();
    symlink("src", b.join("link-to-src")).unwrap();
    symlink("main.ts", b.join("src/link.ts")).unwrap();
    fs::create_dir(tmp.join("outside")).unwrap();
    fs::write(tmp.join("outside/secret.txt"), "outside\n").unwrap();
    symlink("../outside", b.join("escape")).unwrap();
    fs::write(b.join("big.txt"), "abcdefghi\n".repeat(30 * 1024)).unwrap();
    fs::write(
        b.join("huge.txt"),
        format!("{}\n", "x".repeat(99)).repeat(25 * 1024),
    )
    .unwrap();
    fs::write(
        b.join("web/huge.html"),
        format!("<html>\n{}", "<p>paragraph</p>\n".repeat(130 * 1024)),
    )
    .unwrap();
    fs::create_dir(tmp.join("worktree")).unwrap();
    fs::write(tmp.join("worktree/.git"), "gitdir: ../wt-gitdir\n").unwrap();
    fs::write(tmp.join("worktree/a.txt"), "a\n").unwrap();
    fs::create_dir(tmp.join("wt-gitdir")).unwrap();
    fs::write(
        tmp.join("wt-gitdir/HEAD"),
        "0123456789abcdef0123456789abcdef01234567\n",
    )
    .unwrap();
}

struct Tmp {
    _dir: tempfile::TempDir,
    path: PathBuf,
    s: String,
}

fn new_tmp() -> Tmp {
    let dir = tempfile::Builder::new()
        .prefix("ctx-parity-")
        .tempdir()
        .unwrap();
    let path = dunce::canonicalize(dir.path()).unwrap();
    materialize(&path);
    let s = path.to_string_lossy().into_owned();
    Tmp { _dir: dir, path, s }
}

impl Tmp {
    fn sub(&self, s: &str) -> String {
        s.replace("<TMP>", &self.s)
    }

    /// Mirrors `normalize` in the recorder.
    fn normalize(&self, v: Value) -> Value {
        match v {
            Value::String(s) => Value::String(s.replace(&self.s, "<TMP>")),
            Value::Array(a) => Value::Array(a.into_iter().map(|v| self.normalize(v)).collect()),
            Value::Object(o) => {
                let mut out = Map::new();
                for (k, v) in o {
                    match k.as_str() {
                        "mtimeMs" => {
                            out.insert(k, json!("<mtime>"));
                        }
                        "backupId" if v.is_string() => {
                            out.insert(k, json!("<backupId>"));
                        }
                        "places" => {}
                        _ => {
                            out.insert(k, self.normalize(v));
                        }
                    }
                }
                Value::Object(out)
            }
            v => v,
        }
    }

    fn outcome<T: serde::Serialize>(&self, r: Result<T, Error>) -> Value {
        match r {
            Ok(v) => json!({ "result": self.normalize(serde_json::to_value(v).unwrap()) }),
            Err(e) => json!({ "error": self.normalize(error_json(&e)) }),
        }
    }
}

fn error_json(e: &Error) -> Value {
    let status = match e.code {
        ErrorCode::InvalidInput => 400,
        ErrorCode::Forbidden if e.message.starts_with("Cannot read folder") => 403,
        ErrorCode::Forbidden => 400,
        ErrorCode::NotFound => 404,
        ErrorCode::Conflict => 409,
        _ => 500,
    };
    json!({ "status": status, "message": e.message })
}

/// The recorded `{ result }` or `{ error }` part of a case.
fn expected_outcome(case: &Value) -> Value {
    match case.get("error") {
        Some(e) => json!({ "error": e }),
        None => json!({ "result": case["result"] }),
    }
}

#[test]
fn scan_matches_node() {
    let tmp = new_tmp();
    for case in load("scan.json").as_array().unwrap() {
        let input = tmp.sub(case["input"].as_str().unwrap());
        let options: ScanOptions = serde_json::from_value(case["options"].clone()).unwrap();
        let mut got = tmp.outcome(scan::scan_project(&input, &options));
        // Additive workspace metadata did not exist in the Node recordings.
        // Repository discovery and nested ignores have their own regression tests.
        if let Some(result) = got.get_mut("result").and_then(Value::as_object_mut) {
            result.remove("repositories");
        }
        assert_eq!(got, expected_outcome(case), "scan {}", case["input"]);
    }
}

#[test]
fn read_matches_node() {
    let tmp = new_tmp();
    for case in load("read.json").as_array().unwrap() {
        let root = tmp.sub(case["root"].as_str().unwrap());
        let paths: Vec<String> = serde_json::from_value(case["paths"].clone()).unwrap();
        let opts = ReadOptions {
            condense: case["condense"].as_bool().unwrap(),
        };
        let got = tmp.outcome(Ok::<_, Error>(files::read_files(&root, &paths, opts)));
        assert_eq!(
            got,
            expected_outcome(case),
            "read {} {:?}",
            case["root"],
            paths
        );
    }
}

#[test]
fn browse_matches_node() {
    let tmp = new_tmp();
    for case in load("browse.json").as_array().unwrap() {
        let input = tmp.sub(case["input"].as_str().unwrap());
        let r = browse::list_dirs(Some(&input), case["hidden"].as_bool().unwrap());
        if let Ok(listing) = &r {
            assert_eq!(listing.places, platform::places());
        }
        assert_eq!(
            tmp.outcome(r),
            expected_outcome(case),
            "browse {}",
            case["input"]
        );
    }
}

fn b64(data: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(data)
}

/// Mirrors `state()` in the recorder.
fn state(root: &Path, tracked: &[String], tracked_dirs: &[String]) -> Value {
    let files: Map<String, Value> = tracked
        .iter()
        .map(|t| {
            (
                t.clone(),
                fs::read(root.join(t)).map_or(Value::Null, |b| Value::String(b64(&b))),
            )
        })
        .collect();
    let dirs: Map<String, Value> = tracked_dirs
        .iter()
        .map(|d| (d.clone(), Value::Bool(root.join(d).exists())))
        .collect();
    json!({ "files": files, "dirs": dirs })
}

/// Mirrors `snapshots()` in the recorder.
fn snapshots(undo_base: &Path, root: &Path) -> Value {
    fn walk(dir: &Path, rel: &str, out: &mut Map<String, Value>) {
        let Ok(rd) = fs::read_dir(dir) else { return };
        let mut entries: Vec<_> = rd.map(|e| e.unwrap()).collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let name = e.file_name().to_string_lossy().into_owned();
            let r = if rel.is_empty() {
                name
            } else {
                format!("{rel}/{name}")
            };
            if e.file_type().unwrap().is_dir() {
                walk(&e.path(), &r, out);
            } else {
                out.insert(r, Value::String(b64(&fs::read(e.path()).unwrap())));
            }
        }
    }
    let mut list = Vec::new();
    let Ok(hashes) = fs::read_dir(undo_base) else {
        return json!([]);
    };
    let expected_hash = &files::sha1_hex(root.to_string_lossy().as_bytes())[..16];
    for hash in hashes {
        let hash = hash.unwrap();
        let mut ids: Vec<_> = fs::read_dir(hash.path())
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        ids.sort();
        for dir in ids {
            let mut saved = Map::new();
            walk(&dir.join("files"), "", &mut saved);
            let manifest = fs::read(dir.join("manifest.json"))
                .map_or(Value::Null, |b| serde_json::from_slice(&b).unwrap());
            list.push(json!({
                "hashMatches": hash.file_name().to_string_lossy() == *expected_hash,
                "manifest": manifest,
                "files": saved,
            }));
        }
    }
    Value::Array(list)
}

#[test]
fn apply_and_undo_match_node() {
    let tmp = new_tmp();
    let recorded = load("apply.json");
    let undo_base = tmp.path.join("cache/contextual/undo");
    let root = tmp.path.join("basic");
    let root_s = root.to_string_lossy().into_owned();
    let tracked: Vec<String> = serde_json::from_value(recorded["tracked"].clone()).unwrap();
    let tracked_dirs: Vec<String> =
        serde_json::from_value(recorded["trackedDirs"].clone()).unwrap();

    for (s, hex) in recorded["sha1"].as_object().unwrap() {
        assert_eq!(
            files::sha1_hex(s.as_bytes()),
            hex.as_str().unwrap(),
            "sha1 of {s:?}"
        );
    }

    let mut last_id: Option<String> = None;
    for (i, step) in recorded["steps"].as_array().unwrap().iter().enumerate() {
        let op = step["op"].as_str().unwrap();
        let got = match op {
            "apply" => {
                let changes: Vec<ApplyChange> =
                    serde_json::from_value(step["changes"].clone()).unwrap();
                let r = files::apply_changes_in(
                    &undo_base,
                    &root_s,
                    &changes,
                    step["backup"].as_bool().unwrap(),
                );
                if let Ok(res) = &r {
                    last_id = res.backup_id.clone();
                }
                tmp.outcome(r)
            }
            "undo" => {
                let id = match step["backupId"].as_str().unwrap() {
                    "$last" => last_id.clone().unwrap_or_default(),
                    id => id.to_string(),
                };
                tmp.outcome(files::undo_apply_in(&undo_base, &root_s, &id))
            }
            "write" => {
                fs::write(
                    root.join(step["path"].as_str().unwrap()),
                    step["content"].as_str().unwrap(),
                )
                .unwrap();
                json!({})
            }
            other => panic!("unknown op {other}"),
        };

        let eisdir = step["error"]["message"]
            .as_str()
            .is_some_and(|m| m.starts_with("EISDIR"));
        if eisdir {
            // Documented deviation: refused up front instead of failing half-way.
            assert_eq!(
                got,
                json!({ "error": { "status": 409, "message": "Nothing was saved. src is a folder." } })
            );
        } else if op != "write" {
            assert_eq!(got, expected_outcome(step), "step {i} ({op})");
        }
        assert_eq!(
            state(&root, &tracked, &tracked_dirs),
            step["after"],
            "files after step {i} ({op})"
        );
        if !eisdir {
            assert_eq!(
                tmp.normalize(snapshots(&undo_base, &root)),
                step["snapshots"],
                "snapshots after step {i} ({op})"
            );
        }
    }
}
