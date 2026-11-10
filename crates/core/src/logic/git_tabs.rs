//! Center-tab ids for Source Control diffs (`lib/git-tabs.ts`): `git:staged:<path>` (HEAD → index)
//! or `git:changes:<path>` (index → file). Paths are workspace-relative, so identical
//! filenames in sibling repositories produce distinct tabs.

use contextual_protocol::fs::Repository;
use serde::{Deserialize, Serialize};

use super::js;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitTab {
    pub staged: bool,
    pub path: String,
}

pub fn git_tab_id(path: &str, staged: bool) -> String {
    format!("git:{}:{path}", if staged { "staged" } else { "changes" })
}

pub fn parse_git_tab(tab: &str) -> Option<GitTab> {
    let rest = tab.strip_prefix("git:")?;
    let (staged, path) = match rest.strip_prefix("staged:") {
        Some(p) => (true, p),
        None => (false, rest.strip_prefix("changes:")?),
    };
    // `(.+)$`: at least one character and no line terminators.
    if path.is_empty() || path.chars().any(js::is_line_terminator) {
        return None;
    }
    Some(GitTab {
        staged,
        path: path.to_owned(),
    })
}

/// Convert a repository-relative Git path to a workspace path without using the client's
/// OS path rules (the repository may live in WSL).
pub fn workspace_git_path(repository: &Repository, path: &str) -> String {
    if repository.path.is_empty() {
        path.to_string()
    } else {
        format!("{}/{path}", repository.path)
    }
}

/// Resolve a workspace file to its deepest discovered repository and a Git-relative path.
/// An exact repository directory belongs to its parent repository as a gitlink, not to
/// the child as an empty filename.
pub fn repository_file<'a>(
    repositories: &'a [Repository],
    path: &'a str,
) -> Option<(&'a Repository, &'a str)> {
    if path.contains('\0') || path.split('/').any(|part| matches!(part, "" | "." | "..")) {
        return None;
    }
    repositories
        .iter()
        .filter_map(|repository| {
            let relative = if repository.path.is_empty() {
                path
            } else {
                path.strip_prefix(&repository.path)?.strip_prefix('/')?
            };
            (!relative.is_empty()).then_some((repository, relative))
        })
        .max_by_key(|(repository, _)| repository.path.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repository_paths_are_scoped_and_use_component_boundaries() {
        let repositories = vec![
            Repository {
                root: "/workspace/frontend".into(),
                path: "frontend".into(),
                branch: None,
            },
            Repository {
                root: "/workspace/backend".into(),
                path: "backend".into(),
                branch: None,
            },
        ];
        let (repository, relative) = repository_file(&repositories, "backend/src/main.py").unwrap();
        assert_eq!(repository.root, "/workspace/backend");
        assert_eq!(relative, "src/main.py");
        assert_eq!(
            workspace_git_path(repository, relative),
            "backend/src/main.py"
        );
        assert!(repository_file(&repositories, "backend-other/src/main.py").is_none());
        assert!(repository_file(&repositories, "backend/../frontend/main.py").is_none());
        assert_ne!(
            git_tab_id(&workspace_git_path(&repositories[0], "README.md"), false),
            git_tab_id(&workspace_git_path(&repositories[1], "README.md"), false)
        );
    }

    #[test]
    fn nested_files_use_the_child_but_gitlinks_use_the_parent() {
        let repositories = vec![
            Repository {
                root: "/workspace".into(),
                path: String::new(),
                branch: None,
            },
            Repository {
                root: "/workspace/backend".into(),
                path: "backend".into(),
                branch: None,
            },
        ];
        assert_eq!(
            repository_file(&repositories, "backend/main.py")
                .unwrap()
                .0
                .root,
            "/workspace/backend"
        );
        assert_eq!(
            repository_file(&repositories, "backend").unwrap().0.root,
            "/workspace"
        );
    }

    #[test]
    fn round_trip() {
        let id = git_tab_id("a:b.ts", false);
        assert_eq!(
            parse_git_tab(&id),
            Some(GitTab {
                staged: false,
                path: "a:b.ts".into()
            })
        );
        assert_eq!(parse_git_tab("git:staged:"), None);
        assert_eq!(parse_git_tab("preview:x"), None);
    }
}
