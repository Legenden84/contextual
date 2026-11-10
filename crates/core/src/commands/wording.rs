//! The standing safety wording, in one place so that the status bar, the welcome screen and
//! the documentation say the same thing.
//!
//! The wording is scoped on purpose. It says what is true of file proposals and what is true
//! of commands, and promises neither that every write can be undone nor that nothing is ever
//! written before Save.

/// True whether or not commands can run.
pub const FILES_NOTE: &str = "File proposals are saved only with Save.";
/// Added wherever commands can run.
pub const COMMANDS_NOTE: &str =
    "Commands run only with your approval; their effects are not covered by Save/Undo.";

/// What the app says about writing to disk, for a project where commands can or cannot run.
pub fn scope_note(commands: bool) -> String {
    if commands {
        format!("{FILES_NOTE} {COMMANDS_NOTE}")
    } else {
        FILES_NOTE.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_scoped_wording_is_exact_and_makes_no_blanket_promise() {
        assert_eq!(
            scope_note(true),
            "File proposals are saved only with Save. Commands run only with your approval; \
             their effects are not covered by Save/Undo."
        );
        assert_eq!(
            scope_note(false),
            "File proposals are saved only with Save."
        );
        for text in [scope_note(true), scope_note(false)] {
            let lower = text.to_lowercase();
            for promise in [
                "read-only",
                "never",
                "always",
                "every",
                "nothing is written",
            ] {
                assert!(!lower.contains(promise), "{promise} in {text}");
            }
        }
    }
}
