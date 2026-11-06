//! Headless Contextual: everything except the UI.
//!
//! - [`workspace`]: project-file operations (browse, scan, read, apply/undo, git, PRs). Runs on the
//!   workspace host — in-process for local projects, inside the WSL agent for WSL projects.
//! - [`llm`]: Azure AI Foundry (Responses API with chat-completions fallback), edit repair, drafts.
//! - [`logic`]: pure functions ported from `src/web/lib` (diff, response parsing, edits, prompt…).
//! - [`threads`], [`settings`]: app-side persistence.

pub mod error;
pub mod llm;
pub mod logic;
pub mod platform;
pub mod process;
pub mod settings;
pub mod threads;
pub mod workspace;

pub use error::{Error, Result};
