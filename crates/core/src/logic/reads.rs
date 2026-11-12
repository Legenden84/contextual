//! The model picks its own context: a new chat starts with the project tree and an index of the
//! skills; the model asks for files, folder listings and skills with a `read` block, and the app
//! answers with them (within [`Limits`]) before the model replies for real.
//!
//! Everything here is pure; the app runs the rounds (`state/chat.rs`).

use std::collections::{BTreeSet, HashSet};

use super::format::fmt_k;
use super::markdown::{Block, parse_markdown};
use super::types::{NodeKind, Rule, TreeNode};

/// How the model asks for context. Appended to the system prompt when the model picks files.
pub const READ_FORMAT: &str = "# Reading project files

You are not sent the project's files up front. The first message lists the project tree (paths with approximate token counts) and the skills available below. Before you answer, ask for what you need with a read block, then stop and wait:

```read
src/path/to/File.cs
src/some/folder/
skills/frontend-design/SKILL.md
```

- One path per line, exactly as listed. A path ending in / asks for that folder's listing (for folders shown as folded).
- Ask for every file you will change, and the files you need to understand them. Prefer a few relevant files over many.
- When you ask for files, reply with only the read block: no answer, no guesses about the contents. You get the files in the next message.
- You may ask again after reading, a few times per request. When you have enough, answer normally.
- Only change files you have been sent in full.
- Load a skill (its SKILL.md) when the task matches its description, and follow it. Files a skill refers to are relative to its folder. Skills cannot run commands or scripts here: you only get their instructions.";

/// How much the model may read for one message of the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Files read automatically per message.
    pub files: usize,
    /// Tokens read automatically per message.
    pub tokens: u64,
    /// Read rounds per message.
    pub rounds: usize,
}

pub const LIMITS: Limits = Limits {
    files: 25,
    tokens: 150_000,
    rounds: 5,
};

/// Lines of the project tree before large folders are folded.
pub const TREE_LINES: usize = 1500;
/// Lines of one requested folder listing.
pub const LISTING_LINES: usize = 600;

/// What a reply asks for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReadRequest {
    pub paths: Vec<String>,
    /// Folders whose listing is asked for (written with a trailing `/`).
    pub folders: Vec<String>,
}

impl ReadRequest {
    pub fn is_empty(&self) -> bool {
        self.paths.is_empty() && self.folders.is_empty()
    }
}

pub(crate) fn clean_path(line: &str) -> Option<String> {
    let line = line
        .trim()
        .trim_start_matches(['-', '*'])
        .trim()
        .trim_matches('`')
        .trim();
    let line = line.strip_prefix("./").unwrap_or(line).replace('\\', "/");
    (!line.is_empty() && !line.starts_with('#')).then_some(line)
}

/// The `read` blocks of a reply (every one, merged), if any.
pub fn parse_read_request(reply: &str) -> Option<ReadRequest> {
    let mut req = ReadRequest::default();
    let mut found = false;
    let mut seen = HashSet::new();
    for line_block in read_blocks(reply) {
        found = true;
        for line in line_block.lines() {
            let Some(p) = clean_path(line) else { continue };
            if !seen.insert(p.clone()) {
                continue;
            }
            match p.strip_suffix('/') {
                Some(folder) => req.folders.push(folder.to_string()),
                None => req.paths.push(p),
            }
        }
    }
    found.then_some(req)
}

/// Bodies of ```` ```read ```` fences.
fn read_blocks(reply: &str) -> Vec<String> {
    fenced_blocks(reply, "read")
}

/// Bodies of the fences whose info string is `info`.
pub(crate) fn fenced_blocks(reply: &str, info: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current: Option<Vec<&str>> = None;
    for line in reply.lines() {
        let t = line.trim();
        match &mut current {
            None => {
                if let Some(found) = t.strip_prefix("```")
                    && found.trim().eq_ignore_ascii_case(info)
                {
                    current = Some(Vec::new());
                }
            }
            Some(body) => {
                if t.starts_with("```") {
                    out.push(body.join("\n"));
                    current = None;
                } else {
                    body.push(line);
                }
            }
        }
    }
    if let Some(body) = current {
        out.push(body.join("\n"));
    }
    out
}

/// The reply without its `read` blocks and anything after the first one (the model was told to
/// stop there), for showing in the chat.
pub fn without_read_blocks(reply: &str) -> String {
    let mut out = Vec::new();
    for line in reply.lines() {
        if line
            .trim()
            .strip_prefix("```")
            .is_some_and(|i| i.trim().eq_ignore_ascii_case("read"))
        {
            break;
        }
        out.push(line);
    }
    out.join("\n").trim().to_string()
}

/// Whether a parsed reply has anything besides the read request (prose is fine; changes aren't).
pub fn proposes_changes(reply: &str) -> bool {
    parse_markdown(reply).iter().any(|b| {
        matches!(
            b,
            Block::Edit { .. } | Block::Delete { .. } | Block::Code { path: Some(_), .. }
        )
    })
}

// ------------------------------------------------------------------ tree

fn sum_tokens(node: &TreeNode) -> (u64, usize) {
    match &node.children {
        Some(children) => children
            .iter()
            .filter(|c| c.excluded.is_none())
            .map(sum_tokens)
            .fold((0, 0), |a, b| (a.0 + b.0, a.1 + b.1)),
        None => (node.tokens.unwrap_or(0), 1),
    }
}

fn visible(nodes: &[TreeNode]) -> impl Iterator<Item = &TreeNode> {
    nodes.iter().filter(|n| n.excluded.is_none())
}

/// Folders (by path) whose children are listed: breadth first, while within `max_lines`.
fn expanded_folders(nodes: &[TreeNode], max_lines: usize) -> HashSet<String> {
    let mut lines = visible(nodes).count();
    let mut open = HashSet::new();
    let mut queue: std::collections::VecDeque<&TreeNode> =
        visible(nodes).filter(|n| n.kind == NodeKind::Dir).collect();
    while let Some(dir) = queue.pop_front() {
        let children: Vec<&TreeNode> =
            visible(dir.children.as_deref().unwrap_or_default()).collect();
        if lines + children.len() > max_lines {
            continue;
        }
        lines += children.len();
        open.insert(dir.path.clone());
        queue.extend(children.into_iter().filter(|c| c.kind == NodeKind::Dir));
    }
    open
}

fn render(nodes: &[TreeNode], open: &HashSet<String>, depth: usize, out: &mut Vec<String>) {
    let pad = "  ".repeat(depth);
    for n in visible(nodes) {
        match n.kind {
            NodeKind::Dir => {
                let (tokens, files) = sum_tokens(n);
                if open.contains(&n.path) {
                    out.push(format!("{pad}{}/ ({})", n.name, fmt_k(tokens)));
                    render(
                        n.children.as_deref().unwrap_or_default(),
                        open,
                        depth + 1,
                        out,
                    );
                } else {
                    out.push(format!(
                        "{pad}{}/ (folded: {files} {}, {})",
                        n.name,
                        if files == 1 { "file" } else { "files" },
                        fmt_k(tokens)
                    ));
                }
            }
            NodeKind::File => {
                let size = if n.image == Some(true) {
                    "image".to_string()
                } else {
                    fmt_k(n.tokens.unwrap_or(0))
                };
                out.push(format!("{pad}{} {size}", n.name));
            }
        }
    }
}

/// The project tree as the model sees it: excluded entries left out, large folders folded.
pub fn project_tree(nodes: &[TreeNode], max_lines: usize) -> String {
    let open = expanded_folders(nodes, max_lines);
    let mut lines = Vec::new();
    render(nodes, &open, 0, &mut lines);
    lines.join("\n")
}

fn find<'a>(nodes: &'a [TreeNode], path: &str) -> Option<&'a TreeNode> {
    nodes.iter().find_map(|n| {
        if n.path == path {
            Some(n)
        } else if path.starts_with(&format!("{}/", n.path)) {
            find(n.children.as_deref().unwrap_or_default(), path)
        } else {
            None
        }
    })
}

/// One folder's listing (paths relative to it), or `None` when it isn't a visible folder.
pub fn folder_listing(nodes: &[TreeNode], path: &str, max_lines: usize) -> Option<String> {
    let dir = find(nodes, path).filter(|n| n.kind == NodeKind::Dir && n.excluded.is_none())?;
    let children = dir.children.as_deref().unwrap_or_default();
    Some(project_tree(children, max_lines))
}

/// The first user message's tree section.
pub fn tree_section(tree_text: &str) -> String {
    format!(
        "# Project files\n\nPaths with approximate tokens; excluded files are left out. Ask for what you need with a read block.\n\n```\n{tree_text}\n```"
    )
}

// ------------------------------------------------------------------ skills

/// A skill's name and description: from its frontmatter, else its first heading and paragraph.
pub fn skill_summary(path: &str, content: &str) -> (String, Option<String>) {
    let folder = path
        .rsplit('/')
        .nth(1)
        .map(str::to_string)
        .unwrap_or_else(|| path.to_string());
    let mut name = None;
    let mut description = None;
    let mut body = content;
    if let Some(rest) = content.strip_prefix("---")
        && let Some(end) = rest.find("\n---")
    {
        for line in rest[..end].lines() {
            if let Some((k, v)) = line.split_once(':') {
                let v = v.trim().trim_matches(['"', '\'']).trim();
                match k.trim() {
                    "name" if !v.is_empty() => name = Some(v.to_string()),
                    "description" if !v.is_empty() && v != ">" && v != "|" => {
                        description = Some(v.to_string())
                    }
                    _ => {}
                }
            }
        }
        body = &rest[end + 4..];
    }
    if description.is_none() {
        // The first paragraph that isn't a heading.
        description = body
            .split("\n\n")
            .map(str::trim)
            .find(|p| !p.is_empty() && !p.starts_with('#'))
            .map(|p| p.split_whitespace().collect::<Vec<_>>().join(" "));
    }
    let description = description.map(|d| {
        if d.chars().count() > 300 {
            format!("{}…", d.chars().take(300).collect::<String>())
        } else {
            d
        }
    });
    (name.unwrap_or(folder), description)
}

/// The skills the model may load, for the system prompt.
pub fn skill_index(skills: &[&Rule]) -> String {
    if skills.is_empty() {
        return String::new();
    }
    let lines: Vec<String> = skills
        .iter()
        .map(|s| {
            let (name, description) = skill_summary(&s.path, &s.content);
            format!(
                "- {name} ({}, {} tokens): {}",
                s.path,
                fmt_k(s.tokens),
                description.as_deref().unwrap_or("no description")
            )
        })
        .collect();
    format!(
        "# Available skills\n\nLoad one with a read block (its SKILL.md path) when the task matches.\n\n{}",
        lines.join("\n")
    )
}

/// The folder of a skill (`a/b/SKILL.md` → `a/b`).
pub fn skill_dir(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(d, _)| d)
}

// ------------------------------------------------------------------ resolving a request

/// Why a requested path wasn't sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refused {
    pub path: String,
    pub reason: String,
}

/// A request checked against the project.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Resolved {
    /// Project files to read and add to the selection.
    pub files: Vec<String>,
    /// Skills (`SKILL.md`) and files inside the folders of skills.
    pub skill_files: Vec<String>,
    /// Folder listings, as (path, listing).
    pub listings: Vec<(String, String)>,
    /// Already sent and unchanged: not read again.
    pub already: Vec<String>,
    pub refused: Vec<Refused>,
    /// Estimated tokens of `files` and `skill_files`.
    pub tokens: u64,
}

impl Resolved {
    pub fn reads(&self) -> usize {
        self.files.len() + self.skill_files.len()
    }

    pub fn is_empty(&self) -> bool {
        self.reads() == 0 && self.listings.is_empty()
    }
}

/// Checks a request: files must be in the tree and not excluded; skills and their own files are
/// allowed unless turned off. `sent` are the paths the model already has.
pub fn resolve(
    req: &ReadRequest,
    tree: &[TreeNode],
    skills: &[Rule],
    never: &HashSet<String>,
    sent: &BTreeSet<String>,
) -> Resolved {
    let mut r = Resolved::default();
    let refuse = |r: &mut Resolved, path: &str, reason: &str| {
        r.refused.push(Refused {
            path: path.to_string(),
            reason: reason.to_string(),
        })
    };
    let skill_dirs: Vec<(&str, &Rule)> = skills
        .iter()
        .filter(|s| !never.contains(&s.path))
        .map(|s| (skill_dir(&s.path), s))
        .collect();
    for path in &req.paths {
        if sent.contains(path) {
            r.already.push(path.clone());
            continue;
        }
        if let Some(skill) = skills.iter().find(|s| &s.path == path) {
            if never.contains(&skill.path) {
                refuse(&mut r, path, "turned off by the user");
            } else {
                r.tokens += skill.tokens;
                r.skill_files.push(path.clone());
            }
            continue;
        }
        if skill_dirs
            .iter()
            .any(|(d, _)| !d.is_empty() && path.starts_with(&format!("{d}/")))
        {
            if path.split('/').any(|seg| seg == "..") {
                refuse(&mut r, path, "not a project path");
            } else {
                // Size unknown until read; counted as read.
                r.skill_files.push(path.clone());
            }
            continue;
        }
        match find(tree, path) {
            None => refuse(&mut r, path, "not found"),
            Some(n) if n.kind == NodeKind::Dir => {
                // A folder named without the trailing slash: list it.
                match folder_listing(tree, path, LISTING_LINES) {
                    Some(l) => r.listings.push((path.clone(), l)),
                    None => refuse(&mut r, path, "excluded"),
                }
            }
            Some(n) => match &n.excluded {
                Some(reason) => refuse(&mut r, path, &format!("excluded ({reason})")),
                None => {
                    r.tokens += n.tokens.unwrap_or(0);
                    r.files.push(path.clone());
                }
            },
        }
    }
    for folder in &req.folders {
        match folder_listing(tree, folder, LISTING_LINES) {
            Some(l) => r.listings.push((folder.clone(), l)),
            None => refuse(&mut r, &format!("{folder}/"), "not a folder in the tree"),
        }
    }
    r
}

/// Splits a request at the per-message limits: what may be read now, and what needs the user.
pub fn within_limits(
    mut r: Resolved,
    files_so_far: usize,
    tokens_so_far: u64,
    limits: Limits,
) -> (Resolved, Option<Resolved>) {
    if files_so_far + r.reads() <= limits.files && tokens_so_far + r.tokens <= limits.tokens {
        return (r, None);
    }
    let mut over = Resolved::default();
    let (mut files, mut tokens) = (files_so_far, tokens_so_far);
    let fits =
        |files: usize, tokens: u64, add: u64| files < limits.files && tokens + add <= limits.tokens;
    let mut keep = |list: Vec<String>, size: &dyn Fn(&str) -> u64, out_over: &mut Vec<String>| {
        let mut kept = Vec::new();
        for p in list {
            let t = size(&p);
            if fits(files, tokens, t) {
                files += 1;
                tokens += t;
                kept.push(p);
            } else {
                out_over.push(p);
            }
        }
        kept
    };
    // Sizes aren't kept per path in `Resolved`; spread its total evenly (good enough to split).
    let per = r.tokens / r.reads().max(1) as u64;
    let size = move |_: &str| per;
    r.skill_files = keep(
        std::mem::take(&mut r.skill_files),
        &size,
        &mut over.skill_files,
    );
    r.files = keep(std::mem::take(&mut r.files), &size, &mut over.files);
    over.tokens = per * over.reads() as u64;
    r.tokens = per * r.reads() as u64;
    let over = (over.reads() > 0).then_some(over);
    (r, over)
}

/// The text of the follow-up turn that answers a read request (the files themselves are attached
/// as context files).
pub fn followup_text(r: &Resolved, held_back: Option<&Resolved>, last_round: bool) -> String {
    let mut parts = answer_parts(r);
    if let Some(h) = held_back {
        parts.push(format!(
            "Not sent, the user declined (too much for one message): {}.",
            held_paths(h)
        ));
    }
    parts.push(if last_round {
        "No more files can be read for this message. Answer the original request with what you have, and say what is missing if that limits the answer.".into()
    } else {
        "Continue with the original request: ask for more with a read block if needed, otherwise answer.".into()
    });
    format!("# Requested context\n\n{}", parts.join("\n\n"))
}

/// What a read request got, part by part (the files themselves are attached as context files).
pub(crate) fn answer_parts(r: &Resolved) -> Vec<String> {
    let mut parts = Vec::new();
    let sent: Vec<&String> = r.files.iter().chain(&r.skill_files).collect();
    if !sent.is_empty() {
        parts.push(format!(
            "Sent above: {}.",
            sent.iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if !r.already.is_empty() {
        parts.push(format!(
            "Already sent earlier and unchanged: {}.",
            r.already.join(", ")
        ));
    }
    for (path, listing) in &r.listings {
        parts.push(format!("Listing of {path}/:\n```\n{listing}\n```"));
    }
    if !r.refused.is_empty() {
        parts.push(format!(
            "Not available: {}.",
            r.refused
                .iter()
                .map(|x| format!("{} ({})", x.path, x.reason))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    parts
}

pub(crate) fn held_paths(h: &Resolved) -> String {
    h.files
        .iter()
        .chain(&h.skill_files)
        .map(|s| s.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

/// The follow-up when a reply changed files the model was never sent.
pub fn unseen_edit_text(paths: &[String]) -> String {
    format!(
        "# Requested context\n\nYou proposed changes to files you had not been sent: {}. They are attached above. Return your changes again, against their actual contents.",
        paths.join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logic::types::RuleType;

    fn file(path: &str, tokens: u64) -> TreeNode {
        TreeNode {
            name: path.rsplit('/').next().unwrap().into(),
            path: path.into(),
            kind: NodeKind::File,
            tokens: Some(tokens),
            ..TreeNode::default()
        }
    }

    fn dir(path: &str, children: Vec<TreeNode>) -> TreeNode {
        TreeNode {
            name: path.rsplit('/').next().unwrap().into(),
            path: path.into(),
            kind: NodeKind::Dir,
            children: Some(children),
            ..TreeNode::default()
        }
    }

    fn tree() -> Vec<TreeNode> {
        let mut env = file(".env", 10);
        env.excluded = Some("secrets".into());
        vec![
            dir(
                "src",
                vec![
                    file("src/main.rs", 1200),
                    dir(
                        "src/ui",
                        (0..30)
                            .map(|i| file(&format!("src/ui/v{i}.rs"), 100))
                            .collect(),
                    ),
                ],
            ),
            file("Cargo.toml", 300),
            env,
        ]
    }

    #[test]
    fn parses_read_blocks_and_strips_them() {
        let reply = "I need these.\n\n```read\nsrc/main.rs\n- `src/ui/`\n./Cargo.toml\nsrc/main.rs\n```\nignored";
        let req = parse_read_request(reply).unwrap();
        assert_eq!(req.paths, vec!["src/main.rs", "Cargo.toml"]);
        assert_eq!(req.folders, vec!["src/ui"]);
        assert_eq!(without_read_blocks(reply), "I need these.");
        assert!(parse_read_request("```rust\nfn x() {}\n```").is_none());
        assert!(!proposes_changes(reply));
        assert!(proposes_changes("### DELETE: src/main.rs\n"));
    }

    #[test]
    fn tree_folds_large_folders_and_hides_excluded() {
        let full = project_tree(&tree(), 100);
        assert!(full.contains("src/ (4.2k)"));
        assert!(full.contains("  ui/ (3k)"));
        assert!(full.contains("    v29.rs 100"));
        assert!(!full.contains(".env"));
        let folded = project_tree(&tree(), 10);
        assert!(folded.contains("  ui/ (folded: 30 files, 3k)"), "{folded}");
        assert!(!folded.contains("v29.rs"));
        let listing = folder_listing(&tree(), "src/ui", 100).unwrap();
        assert!(listing.starts_with("v0.rs 100"));
    }

    #[test]
    fn skills_come_from_frontmatter_or_first_paragraph() {
        let (n, d) = skill_summary(
            ".claude/skills/frontend-design/SKILL.md",
            "---\nname: frontend-design\ndescription: \"Visual polish.\"\n---\n# X\n",
        );
        assert_eq!(
            (n.as_str(), d.as_deref()),
            ("frontend-design", Some("Visual polish."))
        );
        let (n, d) = skill_summary(
            "skills/review/SKILL.md",
            "# Review\n\nChecks the  diff\nfor bugs.\n",
        );
        assert_eq!(
            (n.as_str(), d.as_deref()),
            ("review", Some("Checks the diff for bugs."))
        );
    }

    #[test]
    fn resolves_against_tree_skills_and_limits() {
        let skill = Rule {
            path: "skills/review/SKILL.md".into(),
            kind: RuleType::Skill,
            tokens: 500,
            content: String::new(),
        };
        let req = ReadRequest {
            paths: vec![
                "src/main.rs".into(),
                ".env".into(),
                "nope.rs".into(),
                "Cargo.toml".into(),
                "skills/review/SKILL.md".into(),
                "skills/review/checklist.md".into(),
                "skills/review/../../.env".into(),
            ],
            folders: vec!["src/ui".into()],
        };
        let sent: BTreeSet<String> = ["Cargo.toml".to_string()].into();
        let r = resolve(
            &req,
            &tree(),
            std::slice::from_ref(&skill),
            &HashSet::new(),
            &sent,
        );
        assert_eq!(r.files, vec!["src/main.rs"]);
        assert_eq!(
            r.skill_files,
            vec!["skills/review/SKILL.md", "skills/review/checklist.md"]
        );
        assert_eq!(r.already, vec!["Cargo.toml"]);
        assert_eq!(r.listings.len(), 1);
        let reasons: Vec<&str> = r.refused.iter().map(|x| x.reason.as_str()).collect();
        assert_eq!(
            reasons,
            vec!["excluded (secrets)", "not found", "not a project path"]
        );
        assert_eq!(r.tokens, 1700);

        let never: HashSet<String> = [skill.path.clone()].into();
        let r2 = resolve(&req, &tree(), std::slice::from_ref(&skill), &never, &sent);
        assert!(
            r2.refused
                .iter()
                .any(|x| x.reason == "turned off by the user")
        );

        let limits = Limits {
            files: 2,
            tokens: 10_000,
            rounds: 5,
        };
        let (now, later) = within_limits(r.clone(), 1, 0, limits);
        assert_eq!(now.reads(), 1);
        assert_eq!(later.unwrap().reads(), 2);
        let (now, later) = within_limits(r, 0, 0, LIMITS);
        assert_eq!((now.reads(), later), (3, None));
    }

    #[test]
    fn followup_says_what_was_sent_and_refused() {
        let r = Resolved {
            files: vec!["a.rs".into()],
            refused: vec![Refused {
                path: "b.rs".into(),
                reason: "not found".into(),
            }],
            ..Resolved::default()
        };
        let t = followup_text(&r, None, false);
        assert!(t.contains("Sent above: a.rs."));
        assert!(t.contains("b.rs (not found)"));
        assert!(followup_text(&r, None, true).contains("No more files can be read"));
    }
}
