//! Center-tab ids for Source Control diffs (`lib/git-tabs.ts`): `git:staged:<path>` (HEAD → index)
//! or `git:changes:<path>` (index → file).

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

#[cfg(test)]
mod tests {
    use super::*;

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
