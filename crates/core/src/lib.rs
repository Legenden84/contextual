//! Headless Contextual: everything except the UI.
//!
//! - [`workspace`]: project-file operations (browse, scan, read, apply/undo, git, PRs). Runs on the
//!   workspace host — in-process for local projects, inside the WSL agent for WSL projects.
//! - [`host`]: [`host::WorkspaceHost`], the interface in front of `workspace`, and [`host::LocalHost`];
//!   [`remote::RemoteHost`] forwards the same calls to `contextual-agent`.
//! - [`llm`]: Azure AI Foundry (Responses API with chat-completions fallback), edit repair, drafts.
//! - [`logic`]: pure functions ported from `src/web/lib` (diff, response parsing, edits, prompt…).
//! - [`threads`], [`settings`]: app-side persistence.
//! - [`location`], [`wsl`]: where a project lives; WSL distro discovery (Windows app).

#[cfg(feature = "llm")]
pub mod chat;
pub mod commands;
pub mod error;
pub mod host;
#[cfg(feature = "llm")]
pub mod inspect;
#[cfg(feature = "llm")]
pub mod llm;
pub mod location;
pub mod logic;
#[cfg(feature = "llm")]
pub mod migrate;
#[cfg(feature = "llm")]
pub mod models;
pub mod platform;
pub mod process;
pub mod recents;
pub mod remote;
#[cfg(feature = "llm")]
pub mod settings;
pub mod threads;
pub mod workspace;
pub mod wsl;

pub use error::{Error, Result};
