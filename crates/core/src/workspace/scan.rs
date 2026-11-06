//! Scans an opened project folder into the explorer tree (port of `fs/scan.ts`): token
//! estimates, exclusions (globs, root `.gitignore`, binaries, size, hidden, secrets), the git
//! branch and instruction files (rules and skills).
//!
//! Exclusion globs follow picomatch's options as the old server used them
//! (`{ dot: true, basename: !glob.includes('/') }`): `*` never crosses `/`, dot files match,
//! slash-less globs match the base name, `a/**` also matches `a`, and a leading `!` negates.
//! Globs `globset` can't parse (e.g. extglobs) are ignored. The root `.gitignore` is matched
//! case-insensitively, like the npm `ignore` package's default.

use std::cmp::Ordering;
use std::collections::HashSet;
use std::fs::{self, DirEntry, File, Metadata};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use contextual_protocol::fs::{NodeKind, RuleFile, RuleType, ScanNode, ScanOptions, ScanResult};
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use ignore::gitignore::{Gitignore, GitignoreBuilder};

use super::condense::{CONDENSE_MIN_BYTES, condense, is_condensable, js_round, js_trim};
use super::images::{IMAGE_TOKENS_ESTIMATE, MAX_IMAGE_BYTES, is_image};
use super::paths::{normalize_input_path, path_string, resolve, to_rel};
use crate::error::{Error, Result};
use crate::platform;

/// Larger HTML/SVG files aren't condensed during the scan (same cap as reading).
const MAX_CONDENSE_BYTES: u64 = 2 * 1024 * 1024;
/// Stop listing after this many entries, so a huge folder can't hang the app.
pub const MAX_ENTRIES: usize = 20_000;
/// Never listed or traversed: VCS internals and Contextual's own backups.
const ALWAYS_SKIP: [&str; 2] = [".git", ".contextual"];
/// Hidden entries still shown in the explorer.
const HIDDEN_ALLOWED: [&str; 2] = [".github", ".gitignore"];
/// AI-tool configuration folders (Claude, OpenAI Codex, Cursor, Gemini, DeepSeek…): never shown
/// in the explorer or sent as files, but searched for skills and rules, which usually live here.
pub const AGENT_DIRS: &[&str] = &[
    ".agents",
    ".aider",
    ".amazonq",
    ".augment",
    ".claude",
    ".cline",
    ".codeium",
    ".codex",
    ".continue",
    ".cursor",
    ".deepseek",
    ".gemini",
    ".junie",
    ".kilocode",
    ".kiro",
    ".openai",
    ".opencode",
    ".qwen",
    ".roo",
    ".trae",
    ".windsurf",
];
/// Inside agent folders: tool state that is never searched (Claude Code's worktrees are full
/// copies of the repository).
const AGENT_STATE: &[&str] = &[".claude/worktrees"];

/// An AI-tool folder (by name, case-insensitive).
pub fn is_agent_dir(name: &str) -> bool {
    AGENT_DIRS.iter().any(|d| d.eq_ignore_ascii_case(name))
}
const LOCK_FILES: [&str; 10] = [
    "package-lock.json",
    "yarn.lock",
    "pnpm-lock.yaml",
    "poetry.lock",
    "Pipfile.lock",
    "packages.lock.json",
    "Cargo.lock",
    "composer.lock",
    "Gemfile.lock",
    "bun.lockb",
];
const BINARY_EXT: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "bmp", "ico", "webp", "avif", "tif", "tiff", "psd", "pdf", "zip",
    "gz", "tgz", "7z", "rar", "tar", "jar", "war", "dll", "exe", "pdb", "so", "dylib", "a", "lib",
    "o", "obj", "class", "pyc", "wasm", "woff", "woff2", "ttf", "otf", "eot", "mp3", "mp4", "mov",
    "avi", "webm", "wav", "ogg", "flac", "sqlite", "db", "mdb", "bin", "dat", "nupkg", "snk",
    "pfx",
];
/// Rule files larger than this are skipped.
const MAX_RULE_BYTES: u64 = 200 * 1024;

/// Rough token estimate: about 4 bytes of source per token.
pub fn estimate_tokens(bytes: u64) -> u64 {
    (js_round(bytes as f64 / 4.0) as u64).max(1)
}

/// `.env`, `.env.local`, `.env.production`… but not `.env.example` / `.sample` / `.template`
/// (`/^\.env(?!\.(example|sample|template)$)(\..+)?$/`).
pub fn is_secret_file(name: &str) -> bool {
    match name.strip_prefix(".env") {
        Some("") => true,
        Some(rest) => match rest.strip_prefix('.') {
            Some(ext) => {
                !ext.is_empty()
                    && !ext.contains(['\n', '\r', '\u{2028}', '\u{2029}'])
                    && !matches!(ext, "example" | "sample" | "template")
            }
            None => false,
        },
        None => false,
    }
}

fn is_binary_name(name: &str) -> bool {
    match name.rsplit_once('.') {
        Some((_, ext)) => {
            let ext = ext.to_ascii_lowercase();
            BINARY_EXT.contains(&ext.as_str())
        }
        None => false,
    }
}

/// Modification time in whole milliseconds (`Math.round(stat.mtimeMs)`).
pub(crate) fn mtime_ms(meta: &Metadata) -> Option<i64> {
    let t = meta.modified().ok()?;
    let ms = match t.duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_secs_f64() * 1000.0,
        Err(e) => -(e.duration().as_secs_f64() * 1000.0),
    };
    Some((ms + 0.5).floor() as i64)
}

// ---------------------------------------------------------------------------------------------
// localeCompare
// ---------------------------------------------------------------------------------------------

/// ICU root-collation order of printable ASCII before digits and letters (what `localeCompare`
/// gives on Node), preceded by the whitespace controls.
const PUNCT_ORDER: &str = "\t\n\u{0B}\u{0C}\r _-,;:!?.'\"()[]{}@*/\\&#%`^+<=>|~$";

/// (primary, secondary = accented, tertiary = upper case); `None` for ignorable characters.
fn collation(c: char) -> Option<(u32, u8, u8)> {
    if let Some(i) = PUNCT_ORDER.find(c) {
        return Some((i as u32, 0, 0));
    }
    let base = 100;
    if c.is_ascii_digit() {
        return Some((base + (c as u32 - '0' as u32), 0, 0));
    }
    if c.is_ascii_alphabetic() {
        return Some((
            base + 10 + (c.to_ascii_lowercase() as u32 - 'a' as u32),
            0,
            c.is_ascii_uppercase() as u8,
        ));
    }
    if c.is_control() {
        return None;
    }
    // Latin-1 letters with diacritics sort with their base letter.
    const LATIN1: [(&str, char); 12] = [
        ("àáâãäå", 'a'),
        ("ÀÁÂÃÄÅ", 'A'),
        ("çÇ", 'c'),
        ("èéêë", 'e'),
        ("ÈÉÊË", 'E'),
        ("ìíîï", 'i'),
        ("ÌÍÎÏ", 'I'),
        ("ñÑ", 'n'),
        ("òóôõöø", 'o'),
        ("ÒÓÔÕÖØ", 'O'),
        ("ùúûüýÿ", 'u'),
        ("ÙÚÛÜÝ", 'U'),
    ];
    for (set, b) in LATIN1 {
        if set.contains(c) {
            let b = if matches!(c, 'ý' | 'ÿ') {
                'y'
            } else if c == 'Ý' {
                'Y'
            } else {
                b
            };
            let upper = c.is_uppercase() as u8;
            return collation(b.to_ascii_lowercase()).map(|(p, _, _)| (p, 1, upper));
        }
    }
    let lower = c.to_lowercase().next().unwrap_or(c);
    Some((0x1_0000 + lower as u32, 0, c.is_uppercase() as u8))
}

fn collation_keys(s: &str) -> Vec<(u32, u8, u8)> {
    s.chars().filter_map(collation).collect()
}

/// `a.localeCompare(b, undefined, { sensitivity: 'base' })` (approximation of ICU's root
/// collation: exact for ASCII; ties are broken by code point so the order is deterministic).
pub(crate) fn locale_cmp_base(a: &str, b: &str) -> Ordering {
    let (ka, kb) = (collation_keys(a), collation_keys(b));
    ka.iter()
        .map(|k| k.0)
        .cmp(kb.iter().map(|k| k.0))
        .then_with(|| a.cmp(b))
}

/// `a.localeCompare(b)` (tertiary strength: base letters, then accents, then lower before upper).
pub(crate) fn locale_cmp(a: &str, b: &str) -> Ordering {
    let (ka, kb) = (collation_keys(a), collation_keys(b));
    ka.iter()
        .map(|k| k.0)
        .cmp(kb.iter().map(|k| k.0))
        .then_with(|| ka.iter().map(|k| k.1).cmp(kb.iter().map(|k| k.1)))
        .then_with(|| ka.iter().map(|k| k.2).cmp(kb.iter().map(|k| k.2)))
}

// ---------------------------------------------------------------------------------------------
// Exclusions
// ---------------------------------------------------------------------------------------------

struct ExclusionGlob {
    /// The trimmed glob, as shown in `ignored: <glob>`.
    glob: String,
    set: GlobSet,
    basename: bool,
    negated: bool,
}

impl ExclusionGlob {
    fn new(glob: &str) -> Option<Self> {
        let negated = glob.starts_with('!') && !glob.starts_with("!(");
        let pat = if negated { &glob[1..] } else { glob };
        let mut builder = GlobSetBuilder::new();
        let mut add = |p: &str| -> Option<()> {
            builder.add(
                GlobBuilder::new(p)
                    .literal_separator(true)
                    .backslash_escape(true)
                    .build()
                    .ok()?,
            );
            Some(())
        };
        add(pat)?;
        // picomatch: `a/**` also matches `a` itself.
        if let Some(stem) = pat.strip_suffix("/**").filter(|s| !s.is_empty()) {
            add(stem)?;
        }
        Some(Self {
            glob: glob.to_string(),
            set: builder.build().ok()?,
            basename: !glob.contains('/'),
            negated,
        })
    }

    fn is_match(&self, path: &str) -> bool {
        let target = if self.basename {
            path.rsplit('/').next().unwrap_or(path)
        } else {
            path
        };
        self.set.is_match(target) != self.negated
    }
}

struct Exclusions {
    globs: Vec<ExclusionGlob>,
    gitignore: Option<Gitignore>,
}

impl Exclusions {
    fn new(root: &Path, opts: &ScanOptions) -> Self {
        let globs = opts
            .exclusions
            .iter()
            .map(|g| js_trim(g))
            .filter(|g| !g.is_empty())
            .filter_map(ExclusionGlob::new)
            .collect();
        let gitignore = if opts.respect_gitignore {
            load_gitignore(root)
        } else {
            None
        };
        Self { globs, gitignore }
    }

    /// The exclusion reason for an entry, if any.
    fn reason(
        &self,
        rel: &str,
        name: &str,
        is_dir: bool,
        hidden: bool,
        include_hidden: bool,
    ) -> Option<String> {
        if let Some(hit) = self
            .globs
            .iter()
            .find(|g| g.is_match(rel) || (is_dir && g.is_match(&format!("{rel}/x"))))
        {
            return Some(if is_dir {
                format!("ignored: {name}")
            } else if LOCK_FILES.contains(&name) {
                "lock file".to_string()
            } else {
                format!("ignored: {}", hit.glob)
            });
        }
        if self
            .gitignore
            .as_ref()
            .is_some_and(|gi| gi.matched_path_or_any_parents(rel, is_dir).is_ignore())
        {
            return Some(".gitignore".to_string());
        }
        if !is_dir && is_secret_file(name) {
            return Some("secrets".to_string());
        }
        if !include_hidden && hidden && !HIDDEN_ALLOWED.contains(&name) {
            return Some("hidden".to_string());
        }
        None
    }
}

fn load_gitignore(root: &Path) -> Option<Gitignore> {
    let bytes = fs::read(root.join(".gitignore")).ok()?;
    let text = String::from_utf8_lossy(&bytes);
    let mut builder = GitignoreBuilder::new(root);
    builder.case_insensitive(true).ok()?;
    for line in text.split('\n') {
        // Lines the matcher can't parse are skipped, like `ignore` does.
        let _ = builder.add_line(None, line.strip_suffix('\r').unwrap_or(line));
    }
    builder.build().ok()
}

// ---------------------------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------------------------

fn looks_binary(file: &Path) -> bool {
    let read = || -> std::io::Result<bool> {
        let mut buf = Vec::with_capacity(8000);
        File::open(file)?.take(8000).read_to_end(&mut buf)?;
        Ok(buf.contains(&0))
    };
    read().unwrap_or(true)
}

fn read_text_lossy(path: &Path) -> std::io::Result<String> {
    Ok(String::from_utf8_lossy(&fs::read(path)?).into_owned())
}

fn first_utf16_units(s: &str, n: usize) -> String {
    let mut count = 0;
    s.chars()
        .take_while(|c| {
            count += c.len_utf16();
            count <= n
        })
        .collect()
}

/// The checked-out branch, or the first 7 characters of a detached HEAD.
fn git_branch(root: &Path) -> Option<String> {
    let mut git_dir = root.join(".git");
    if fs::metadata(&git_dir).ok()?.is_file() {
        // Worktree or submodule: ".git" is a file pointing at the real git dir.
        let text = read_text_lossy(&git_dir).ok()?;
        let at = text.find("gitdir:")?;
        let after = text[at + 7..].trim_start_matches(super::condense::is_js_whitespace);
        let line: String = after
            .chars()
            .take_while(|c| !matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}'))
            .collect();
        if line.is_empty() {
            return None;
        }
        git_dir = resolve(root, Path::new(js_trim(&line)));
    }
    let head_text = read_text_lossy(&git_dir.join("HEAD")).ok()?;
    let head = js_trim(&head_text);
    match head.strip_prefix("ref: refs/heads/") {
        Some(branch)
            if !branch.is_empty() && !branch.contains(['\n', '\r', '\u{2028}', '\u{2029}']) =>
        {
            Some(branch.to_string())
        }
        _ => Some(first_utf16_units(head, 7)),
    }
}

fn read_rule(root: &Path, rel: &str, kind: RuleType) -> Option<RuleFile> {
    let abs = root.join(rel);
    let meta = fs::metadata(&abs).ok()?;
    if !meta.is_file() || meta.len() > MAX_RULE_BYTES {
        return None;
    }
    let content = read_text_lossy(&abs).ok()?;
    Some(RuleFile {
        path: rel.to_string(),
        kind,
        tokens: estimate_tokens(content.len() as u64),
        content,
    })
}

fn entry_name(e: &DirEntry) -> String {
    e.file_name().to_string_lossy().into_owned()
}

struct Entry {
    name: String,
    entry: DirEntry,
    is_dir: bool,
    is_file: bool,
    is_symlink: bool,
}

/// `readdir(dir, { withFileTypes: true })`: types of the entries themselves (links not followed).
fn read_entries(dir: &Path) -> Option<Vec<Entry>> {
    let rd = fs::read_dir(dir).ok()?;
    Some(
        rd.filter_map(|e| e.ok())
            .map(|entry| {
                let ft = entry.file_type().ok();
                Entry {
                    name: entry_name(&entry),
                    is_dir: ft.is_some_and(|t| t.is_dir()),
                    is_file: ft.is_some_and(|t| t.is_file()),
                    is_symlink: ft.is_some_and(|t| t.is_symlink()),
                    entry,
                }
            })
            .collect(),
    )
}

// ---------------------------------------------------------------------------------------------
// Rules and skills
// ---------------------------------------------------------------------------------------------

struct Discovered {
    rules: Vec<RuleFile>,
    skill_dirs: HashSet<String>,
    truncated: bool,
}

struct Discovery<'a> {
    root: &'a Path,
    skip_directory: &'a dyn Fn(&str, &str) -> bool,
    candidates: Vec<(String, RuleType)>,
    skill_dirs: HashSet<String>,
    visited: usize,
    truncated: bool,
}

impl Discovery<'_> {
    fn discover(&mut self, dir: &Path) {
        let Some(mut entries) = read_entries(dir) else {
            return;
        };
        entries.sort_by(|a, b| locale_cmp(&a.name, &b.name));
        if let Some(marker) = entries
            .iter()
            .find(|e| e.is_file && e.name.to_lowercase() == "skill.md")
        {
            self.candidates
                .push((to_rel(self.root, &dir.join(&marker.name)), RuleType::Skill));
            // An opened project can itself be a skill; never hide the whole project.
            if dir != self.root {
                self.skill_dirs.insert(to_rel(self.root, dir));
                return;
            }
        }
        for e in &entries {
            if ALWAYS_SKIP.contains(&e.name.as_str()) {
                continue;
            }
            if self.visited >= MAX_ENTRIES {
                self.truncated = true;
                return;
            }
            self.visited += 1;
            // Do not follow symlinks or traverse ignored dependencies/build output.
            if !e.is_dir {
                continue;
            }
            let abs = dir.join(&e.name);
            let rel = to_rel(self.root, &abs);
            if AGENT_STATE.iter().any(|s| rel.eq_ignore_ascii_case(s)) {
                continue;
            }
            if !(self.skip_directory)(&rel, &e.name) {
                self.discover(&abs);
            }
            if self.truncated {
                return;
            }
        }
    }
}

/// Rules use their conventional locations. Skills are discovered by SKILL.md, independently of
/// the framework or parent folder name. Their directories are kept out of file context.
fn find_rules(root: &Path, skip_directory: &dyn Fn(&str, &str) -> bool) -> Discovered {
    let mut candidates: Vec<(String, RuleType)> = vec![
        ("AGENTS.md".into(), RuleType::Rule),
        ("CLAUDE.md".into(), RuleType::Rule),
        (".github/copilot-instructions.md".into(), RuleType::Rule),
    ];
    if let Ok(rd) = fs::read_dir(root.join(".cursor/rules")) {
        let mut names: Vec<String> = rd.filter_map(|e| e.ok()).map(|e| entry_name(&e)).collect();
        names.sort_by(|a, b| super::condense::js_cmp(a, b));
        for n in names {
            let lower = n.to_ascii_lowercase();
            if lower.ends_with(".md") || lower.ends_with(".mdc") {
                candidates.push((format!(".cursor/rules/{n}"), RuleType::Rule));
            }
        }
    }
    let mut d = Discovery {
        root,
        skip_directory,
        candidates,
        skill_dirs: HashSet::new(),
        visited: 0,
        truncated: false,
    };
    d.discover(root);
    let rules = d
        .candidates
        .iter()
        .filter_map(|(rel, kind)| read_rule(root, rel, *kind))
        .collect();
    Discovered {
        rules,
        skill_dirs: d.skill_dirs,
        truncated: d.truncated,
    }
}

// ---------------------------------------------------------------------------------------------
// Scan
// ---------------------------------------------------------------------------------------------

struct Walker<'a> {
    root: PathBuf,
    opts: &'a ScanOptions,
    exclusions: &'a Exclusions,
    skill_dirs: &'a HashSet<String>,
    skill_parents: &'a HashSet<String>,
    max_bytes: f64,
    count: usize,
    files: u64,
    truncated: bool,
}

impl Walker<'_> {
    fn walk(&mut self, dir: &Path) -> Vec<ScanNode> {
        let Some(mut entries) = read_entries(dir) else {
            return Vec::new();
        };
        entries.sort_by(|a, b| {
            b.is_dir
                .cmp(&a.is_dir)
                .then_with(|| locale_cmp_base(&a.name, &b.name))
        });

        let mut out = Vec::new();
        for e in &entries {
            if ALWAYS_SKIP.contains(&e.name.as_str()) || (e.is_dir && is_agent_dir(&e.name)) {
                continue;
            }
            if self.count >= MAX_ENTRIES {
                self.truncated = true;
                break;
            }
            self.count += 1;
            let abs = dir.join(&e.name);
            let rel = to_rel(&self.root, &abs);
            let name = e.name.clone();
            let hidden = platform::is_hidden(&name, Some(&e.entry));

            if e.is_symlink {
                out.push(ScanNode {
                    name,
                    path: rel,
                    kind: NodeKind::File,
                    tokens: None,
                    excluded: Some("symlink".into()),
                    children: None,
                    mtime_ms: None,
                    image: None,
                    condensed_from: None,
                });
                continue;
            }
            if e.is_dir {
                if self.skill_dirs.contains(&rel) {
                    continue;
                }
                let excluded =
                    self.exclusions
                        .reason(&rel, &name, true, hidden, self.opts.include_hidden);
                let children = if excluded.is_some() {
                    Vec::new()
                } else {
                    self.walk(&abs)
                };
                // Remove wrappers made empty by hiding skills, without hiding unrelated config.
                if excluded.is_none() && children.is_empty() && self.skill_parents.contains(&rel) {
                    continue;
                }
                out.push(ScanNode {
                    name,
                    path: rel,
                    kind: NodeKind::Dir,
                    tokens: None,
                    excluded,
                    children: Some(children),
                    mtime_ms: None,
                    image: None,
                    condensed_from: None,
                });
                continue;
            }
            if !e.is_file {
                continue;
            }
            let Ok(meta) = fs::metadata(&abs) else {
                continue;
            };
            out.push(self.file_node(&abs, name, rel, hidden, &meta));
        }
        out
    }

    fn file_node(
        &mut self,
        abs: &Path,
        name: String,
        rel: String,
        hidden: bool,
        meta: &Metadata,
    ) -> ScanNode {
        let size = meta.len();
        let mut excluded =
            self.exclusions
                .reason(&rel, &name, false, hidden, self.opts.include_hidden);
        let image = is_image(&name);
        let mut tokens = if image {
            IMAGE_TOKENS_ESTIMATE
        } else {
            estimate_tokens(size)
        };
        let mut condensed_from = None;

        // Large HTML/SVG: measure the condensed copy, which is what gets sent.
        if excluded.is_none()
            && !image
            && is_condensable(&name)
            && (CONDENSE_MIN_BYTES..=MAX_CONDENSE_BYTES).contains(&size)
        {
            // Unreadable: fall back to the raw size.
            if let Ok(text) = read_text_lossy(abs) {
                condensed_from = Some(tokens);
                tokens = estimate_tokens(condense(&text).len() as u64);
            }
        }

        let sent_bytes = if condensed_from.is_some() {
            tokens * 4
        } else {
            size
        };
        let too_large = || format!("too large ({} KB)", js_round(size as f64 / 1024.0));
        if excluded.is_none() && image && size > MAX_IMAGE_BYTES {
            excluded = Some(too_large());
        }
        if excluded.is_none() && !image && sent_bytes as f64 > self.max_bytes {
            excluded = Some(too_large());
        }
        if excluded.is_none()
            && !image
            && self.opts.skip_binary
            && (is_binary_name(&name) || looks_binary(abs))
        {
            excluded = Some("binary".into());
        }
        if excluded.is_none() {
            self.files += 1;
        }
        ScanNode {
            name,
            path: rel,
            kind: NodeKind::File,
            tokens: Some(tokens),
            excluded,
            children: None,
            mtime_ms: mtime_ms(meta),
            image: image.then_some(true),
            condensed_from,
        }
    }
}

/// Scans the project at `input` (normalised like any typed path).
pub fn scan_project(input: &str, opts: &ScanOptions) -> Result<ScanResult> {
    let root = normalize_input_path(Some(input));
    if !fs::metadata(&root).is_ok_and(|m| m.is_dir()) {
        return Err(Error::not_found(format!(
            "Folder not found: {}",
            path_string(&root)
        )));
    }
    let exclusions = Exclusions::new(&root, opts);

    // Discovery includes hidden framework folders (and the agent folders the explorer leaves
    // out), but still respects explicit exclusions and .gitignore. Skill support files are not
    // ordinary selectable project context.
    let skip = |rel: &str, name: &str| exclusions.reason(rel, name, true, false, true).is_some();
    let discovered = find_rules(&root, &skip);
    let mut skill_parents = HashSet::new();
    for dir in &discovered.skill_dirs {
        let mut parts: Vec<&str> = dir.split('/').collect();
        while parts.len() > 1 {
            parts.pop();
            skill_parents.insert(parts.join("/"));
        }
    }

    let max_kb = if opts.max_file_size_kb > 0.0 {
        opts.max_file_size_kb
    } else {
        256.0
    };
    let mut walker = Walker {
        root: root.clone(),
        opts,
        exclusions: &exclusions,
        skill_dirs: &discovered.skill_dirs,
        skill_parents: &skill_parents,
        max_bytes: max_kb * 1024.0,
        count: 0,
        files: 0,
        truncated: discovered.truncated,
    };
    let tree = walker.walk(&root);
    let root_str = path_string(&root);
    let name = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| root_str.clone());
    Ok(ScanResult {
        name,
        branch: git_branch(&root),
        root: root_str,
        tree,
        rules: discovered.rules,
        files: walker.files,
        truncated: walker.truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_folders_are_hidden_but_searched_for_skills() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let write = |rel: &str, text: &str| {
            let p = root.join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, text).unwrap();
        };
        write("src/main.rs", "fn main() {}\n");
        write(".claude/skills/review/SKILL.md", "# Review\n");
        write(".claude/settings.json", "{}\n");
        write(
            ".claude/worktrees/copy/.claude/skills/review/SKILL.md",
            "# Copy\n",
        );
        write(".claude/worktrees/copy/src/main.rs", "fn main() {}\n");
        write(".codex/skills/ship/SKILL.md", "# Ship\n");
        write(".gemini/notes.md", "notes\n");
        write(".cursor/rules/style.mdc", "Use tabs.\n");
        write(".github/workflows/ci.yml", "on: push\n");

        let scan = scan_project(&root.to_string_lossy(), &ScanOptions::default()).unwrap();
        let top: Vec<&str> = scan.tree.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(
            top,
            vec![".github", "src"],
            "agent folders stay out of the explorer"
        );
        assert_eq!(scan.files, 2);
        let rules: Vec<&str> = scan.rules.iter().map(|r| r.path.as_str()).collect();
        assert!(rules.contains(&".cursor/rules/style.mdc"));
        assert!(rules.contains(&".claude/skills/review/SKILL.md"));
        assert!(rules.contains(&".codex/skills/ship/SKILL.md"));
        assert!(
            !rules.iter().any(|r| r.contains("worktrees")),
            "Claude Code worktrees aren't searched: {rules:?}"
        );
        assert!(!scan.truncated);
        assert!(is_agent_dir(".Claude") && !is_agent_dir(".github"));
    }

    #[test]
    fn secrets() {
        for n in [".env", ".env.local", ".env.production", ".env.example.bak"] {
            assert!(is_secret_file(n), "{n}");
        }
        for n in [
            ".env.example",
            ".env.sample",
            ".env.template",
            ".envrc",
            ".env.",
            "env",
            "a.env",
        ] {
            assert!(!is_secret_file(n), "{n}");
        }
    }

    #[test]
    fn tokens() {
        assert_eq!(estimate_tokens(0), 1);
        assert_eq!(estimate_tokens(2), 1);
        assert_eq!(estimate_tokens(6), 2);
        assert_eq!(estimate_tokens(10), 3);
        assert_eq!(estimate_tokens(4096), 1024);
    }

    #[test]
    fn binary_names() {
        assert!(
            is_binary_name("a.DLL")
                && is_binary_name("x.tar.gz")
                && is_binary_name(".bin")
                && is_binary_name("k.pfx")
        );
        assert!(!is_binary_name("bin") && !is_binary_name("a.rs"));
    }

    #[test]
    fn collation_matches_node() {
        // Recorded from Node 24 (ICU, en-US): sorted with { sensitivity: 'base' }.
        let ascii: Vec<String> = (32u8..127).map(|b| (b as char).to_string()).collect();
        let mut sorted = ascii.clone();
        sorted.sort_by(|a, b| locale_cmp(a, b));
        assert_eq!(
            sorted.concat(),
            " _-,;:!?.'\"()[]{}@*/\\&#%`^+<=>|~$0123456789aAbBcCdDeEfFgGhHiIjJkKlLmMnNoOpPqQrRsStTuUvVwWxXyYzZ"
        );
        assert_eq!(locale_cmp("skill.md", "SKILL.md"), Ordering::Less);
        assert_eq!(locale_cmp("e", "é"), Ordering::Less);
        assert_eq!(locale_cmp_base("a-b", "ab"), Ordering::Less);
        assert_eq!(locale_cmp_base("a\tb", "ab"), Ordering::Less);
        assert_eq!(locale_cmp_base("ab", "a b"), Ordering::Greater);
        assert_eq!(locale_cmp_base("Zeta", "alpha"), Ordering::Greater);
        assert_eq!(locale_cmp_base("file10", "file2"), Ordering::Less);
    }

    #[test]
    fn globs_follow_picomatch() {
        let m = |g: &str, p: &str| ExclusionGlob::new(g).unwrap().is_match(p);
        // Recorded from picomatch 4 with { dot: true, basename: !g.includes('/') }.
        assert!(m("**/bin/**", "bin"));
        assert!(m("**/bin/**", "bin/x"));
        assert!(m("**/bin/**", "a/bin"));
        assert!(m("**/bin/**", "a/bin/x"));
        assert!(m("**/node_modules/**", "node_modules"));
        assert!(m("**/.git/**", ".git/x"));
        assert!(m("package-lock.json", "a/package-lock.json"));
        assert!(m("*.log", "a/.b.log"));
        assert!(!m("src/*.ts", "src/a/b.ts"));
        assert!(!m("!*.md", "a.md"));
        assert!(m("!*.md", "a.ts"));
        assert!(m("*.{js,ts}", "a/b.ts"));
        assert!(m("[ab].txt", "a.txt"));
        assert!(!m("src/", "src"));
        assert!(!m("src", "a/src/x"));
        assert!(!m("**/bin/**", "binary/x"));
    }
}
