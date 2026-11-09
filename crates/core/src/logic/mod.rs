//! Pure logic ported from `src/web/lib` (and the constants and types it needs from
//! `src/web/data`). Behaviour matches the TypeScript exactly; `tests/logic_fixtures.rs` replays
//! golden fixtures recorded from the TS code. Where JavaScript string semantics matter (UTF-16
//! lengths, `\s`, `trim()`, `Math.round`, `toFixed`), the helpers in `js` reproduce them.
//!
//! Offsets exposed to callers are UTF-8 byte offsets into Rust strings (see [`diff::DiffOp::r`]).

pub mod changes;
pub mod composer;
pub mod context;
pub mod defaults;
pub mod diff;
pub mod edits;
pub mod format;
pub mod foundry_status;
pub mod git_tabs;
pub mod hash;
pub mod images;
mod js;
pub mod lang;
pub mod markdown;
pub mod pr;
pub mod prompt;
pub mod reads;
pub mod tree;
pub mod types;
