//! Continuing a reply that was cut off at the completion token limit.
//!
//! The finished sections of a cut-off reply are proposed as usual ([`super::changes::parse_reply`]).
//! The app then sends a short follow-up asking the model to carry on from where it stopped:
//! a file it was writing is continued with an `### APPEND:` section, an edit section with new
//! SEARCH/REPLACE blocks. Each answer is proposed on top of the earlier ones, so a change too
//! long for one reply arrives in several.

use super::changes::{CutSection, Cutoff};
use super::prompt::fence_lang;

/// Follow-up requests for one message after cut-off replies.
pub const MAX_CONTINUATIONS: usize = 5;

/// Lines of a partial file quoted back so the model sees where it stopped.
const TAIL_LINES: usize = 3;

fn code_list(paths: &[String]) -> String {
    paths
        .iter()
        .map(|p| format!("`{p}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The follow-up message that asks the model to continue a cut-off reply. `last`: this is the
/// final continuation the app will ask for.
pub fn continuation_text(cutoff: &Cutoff, last: bool) -> String {
    let mut out = vec![
        "Your previous reply reached the output token limit and was cut off. Continue from where it stopped; do not start over or repeat what was received.".to_owned(),
    ];
    if !cutoff.received.is_empty() {
        out.push(format!(
            "Received complete: {}. Do not send these sections again.",
            code_list(&cutoff.received)
        ));
    }
    match &cutoff.cut {
        Some(CutSection::File(p)) if p.lines() == 0 => out.push(format!(
            "Nothing of `{}` was received. Send it in full with a FILE section.",
            p.path
        )),
        Some(CutSection::File(p)) => {
            let n = p.lines();
            let tail: Vec<&str> = p.content.split('\n').collect();
            let tail = tail[tail.len().saturating_sub(TAIL_LINES)..].join("\n");
            let lang = fence_lang(&p.path);
            out.push(format!(
                "`{path}` was cut off after line {n}; lines 1–{n} were received and end with:\n\n```{lang}\n{tail}\n```\n\nContinue it with an APPEND section holding only the lines after line {n}, which are added to the end of the file:\n\n### APPEND: {path}\n```{lang}\nthe remaining lines\n```",
                path = p.path,
            ));
        }
        Some(CutSection::Edit { path, applied }) => out.push(format!(
            "The EDIT section for `{path}` was cut off. {} Send the remaining SEARCH/REPLACE blocks under a new `### EDIT: {path}` header; their SEARCH text must match the file with the received blocks applied.",
            match applied {
                0 => "None of its blocks were complete.".to_owned(),
                1 => "Its first block was received and applied.".to_owned(),
                n => format!("Its first {n} blocks were received and applied."),
            }
        )),
        None => {}
    }
    out.push(
        "Then finish any remaining changes. Close every section you start; if the reply is cut off again, you will be asked to continue."
            .to_owned(),
    );
    if last {
        out.push(
            "This is the last continuation: finish what you started, and list anything left undone instead of writing it."
                .to_owned(),
        );
    }
    out.join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::super::changes::{ChangeContext, PartialFile, parse_reply};
    use super::*;
    use crate::logic::types::ChangeStatus;

    struct Disk;
    impl ChangeContext for Disk {
        fn exists(&self, path: &str) -> bool {
            path == "a.txt"
        }
        fn original(&self, path: &str) -> Option<String> {
            (path == "a.txt").then(|| "one\ntwo\nthree\n".to_owned())
        }
        fn changed_on_disk(&self, _: &str) -> bool {
            false
        }
    }

    #[test]
    fn finished_sections_survive_a_cut_inside_a_new_file() {
        let reply = "### EDIT: a.txt\n<<<<<<< SEARCH\ntwo\n=======\n2\n>>>>>>> REPLACE\n\n### FILE: b.jsx\n```jsx\nline 1\nline 2\nline 3 is half";
        let parsed = parse_reply(reply, true, None, &Disk);
        assert_eq!(parsed.changes.len(), 1);
        assert_eq!(parsed.changes[0].new, "one\n2\nthree\n");
        let cutoff = parsed.cutoff.unwrap();
        assert_eq!(cutoff.received, vec!["a.txt".to_owned()]);
        assert_eq!(
            cutoff.partial(),
            Some(&PartialFile {
                path: "b.jsx".into(),
                content: "line 1\nline 2".into(),
            })
        );
        let text = continuation_text(&cutoff, false);
        assert!(text.contains("Received complete: `a.txt`"));
        assert!(text.contains("cut off after line 2"));
        assert!(text.contains("### APPEND: b.jsx"));
    }

    #[test]
    fn appends_continue_a_held_file_across_several_replies() {
        let held = PartialFile {
            path: "b.jsx".into(),
            content: "line 1\nline 2".into(),
        };
        // Cut again, inside the append.
        let second = parse_reply(
            "### APPEND: b.jsx\n```jsx\nline 3\nline 4\nli",
            true,
            Some(&held),
            &Disk,
        );
        assert!(second.changes.is_empty());
        let cutoff = second.cutoff.unwrap();
        assert_eq!(cutoff.abandoned, None);
        let held = cutoff.partial().unwrap().clone();
        assert_eq!(held.content, "line 1\nline 2\nline 3\nline 4");

        let third = parse_reply(
            "### APPEND: b.jsx\n```jsx\nline 5\n```\n\nDone.",
            false,
            Some(&held),
            &Disk,
        );
        assert_eq!(third.cutoff, None);
        assert_eq!(third.changes.len(), 1);
        assert_eq!(third.changes[0].status, ChangeStatus::A);
        assert_eq!(
            third.changes[0].new,
            "line 1\nline 2\nline 3\nline 4\nline 5"
        );
    }

    #[test]
    fn a_cut_edit_section_keeps_its_finished_blocks() {
        let reply = "### EDIT: a.txt\n<<<<<<< SEARCH\none\n=======\n1\n>>>>>>> REPLACE\n<<<<<<< SEARCH\nthree\n=======\n3";
        let parsed = parse_reply(reply, true, None, &Disk);
        assert_eq!(parsed.changes[0].new, "1\ntwo\nthree\n");
        let cutoff = parsed.cutoff.unwrap();
        assert!(cutoff.received.is_empty());
        assert_eq!(
            cutoff.cut,
            Some(CutSection::Edit {
                path: "a.txt".into(),
                applied: 1
            })
        );
        assert!(continuation_text(&cutoff, true).contains("first block was received"));
    }

    #[test]
    fn a_header_without_content_asks_for_the_whole_file() {
        let parsed = parse_reply("Intro.\n\n### FILE: c.rs\n", true, None, &Disk);
        let cutoff = parsed.cutoff.unwrap();
        assert_eq!(cutoff.partial().unwrap().lines(), 0);
        assert!(continuation_text(&cutoff, false).contains("Nothing of `c.rs` was received"));
    }

    #[test]
    fn a_held_file_that_is_not_continued_is_abandoned() {
        let held = PartialFile {
            path: "b.jsx".into(),
            content: "line 1".into(),
        };
        let parsed = parse_reply("Sorry, done.", false, Some(&held), &Disk);
        assert_eq!(parsed.cutoff.unwrap().abandoned.as_deref(), Some("b.jsx"));
        // Rewritten in full instead: not abandoned.
        let parsed = parse_reply("### FILE: b.jsx\n```\nall\n```", false, Some(&held), &Disk);
        assert_eq!(parsed.cutoff, None);
        assert_eq!(parsed.changes[0].new, "all");
    }

    #[test]
    fn an_append_to_an_existing_file_adds_after_its_last_line() {
        let parsed = parse_reply("### APPEND: a.txt\n```\nfour\n```", false, None, &Disk);
        assert_eq!(parsed.changes[0].new, "one\ntwo\nthree\nfour");
    }
}
