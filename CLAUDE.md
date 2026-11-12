# Contextual

Local desktop developer tool: a chat client that sends selected project files to a self-hosted
LLM (Azure AI Foundry) and lets the user review and save proposed changes. The model never writes
to disk; saving ("Save" in the UI, `apply` in code) is an explicit, checked, undoable action.
The model can also propose commands, which run only when the user approves the exact command;
their effects are not covered by Save/Undo. Command execution is always configured, with no
Settings flag. Local projects use the native backend and WSL projects use their agent;
unavailable backends, journals or agent capabilities leave execution unavailable.

Several models can be configured (`foundry.models` in settings.json, or `AZURE_OPENAI_<NAME>_…`
variables; endpoint, key and API version are shared). No model is built in: deployment, tier,
limit and prices are what the user imports or types under Settings → Models. The routing model (`foundry.router`,
`AZURE_OPENAI_ROUTER`) then reads each message first — the project tree and the files it asks
for, in a conversation of its own — and names the model that answers and the files it starts
with. Low confidence moves one tier up; a routing that fails stays on the chat's model or goes
to the most capable one. Another model than the chat's last one is sent the whole transcript.
The composer's model control pins a model for a chat instead.

Native Rust app on GPUI for **Windows and macOS**; on Windows it also opens projects inside WSL
through a deployed agent. Architecture, decisions and progress: `docs/plans/cross-platform-rust-port.md`.

## Workspace (`crates/`)
- `core` (`contextual-core`) — everything except the UI; see `crates/core/README.md`.
  - `workspace/` — paths and containment, folder browsing, scan (exclusions, `.gitignore`, skills,
    condensing), read, save/undo (line endings, BOM, final newline kept; pre-checks; atomic writes;
    undo snapshots), git and PRs (shelling out to `git`/`gh` through `process::CommandRunner`).
  - `host.rs` — `WorkspaceHost`, the one interface for project-file operations (`LocalHost`
    in-process with a root allowlist); `remote.rs` — `RemoteHost` (JSON-RPC client) and
    `SupervisedHost` (lazy start, restart with backoff; read-only calls retried, mutations never).
  - `logic/` — pure functions: diff, markdown/response parsing, changes, SEARCH/REPLACE edits,
    `reads` (the model picks files: `read` blocks, project tree, skill index, limits),
    `route` (several models: routing prompt, `route` block, tier rules),
    `cutoff` (replies cut off at the token limit: finished sections kept, follow-up rounds,
    `APPEND` blocks continue a partial file),
    prompt (incl. `FILE_FORMAT`, appended to the system prompt and not shown in Settings), tree
    sums, context totals, images, PR helpers.
  - `chat.rs` — turn preparation (only files new or changed since sent, by hash), saved-thread
    format (reads the old app's JSON; each round keeps its system prompt when it changed, whether
    the app wrote it, usage and, with several models, the model; routing rounds are kept beside
    the transcript), notes, save bookkeeping. `inspect.rs` — the chat round by round
    for the Inspect view. `llm/` — Foundry client on reqwest
    (Responses API with `previous_response_id`, chain reset, chat-completions fallback, SSE),
    drafts and edit repair. `llm`, `settings`, `models`, `chat`, `inspect` and `migrate` sit behind the `llm` feature.
  - `threads.rs`, `settings.rs`, `recents.rs`, `migrate.rs` (`.env.local` import),
    `models.rs` (the user's model descriptions: `models` in settings.json; the app ships
    none), `settings.rs` also resolves the models to route between,
    `location.rs` / `wsl.rs` (where a project lives, distro discovery, agent deployment).
- `protocol` — serde wire types (camelCase, like the old `/api` JSON) and JSON-RPC framing.
- `theme` — `build.rs` generates typed tokens from `docs/design/contextual/tokens.json`.
- `agent` (`contextual-agent`) — stdio JSON-RPC server around `LocalHost`; built static (musl)
  and deployed into WSL distros by the Windows app.
- `app` (`contextual`) — the GPUI app:
  - `state/` — models (GPUI entities): `AppModel` (screen, settings, recents, Foundry, hosts),
    `Project`, `ChatModel`, `ScmModel`, `hosts.rs` (host per location). Business rules stay in core.
  - `views/`, `screens/`, `ui/` (token-styled primitives), `theme.rs` (bridge to GPUI and
    gpui-component), `runtime.rs` (tokio runtime for core futures), `dev.rs` (debug automation).
  - GPUI comes from `gpui-kit = "=0.9.0"` (pins `gpui-pre =0.3.11`); upgrade deliberately.

## Commands
Rust is in `~/.cargo/bin`. Development is Linux-only; the app runs through WSLg.
- `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
- `cargo run -p contextual [-- <project>]` (`--no-restore`, `--settings`, `--spike`).
- Plain `cargo build`/`test` skip `crates/app` (default members); `--workspace` includes it.
- Windows `.exe`: `cargo build -p contextual --target x86_64-pc-windows-gnu`. Type-check core for
  Windows/macOS: `cargo clippy -p contextual-core --no-default-features --target x86_64-pc-windows-msvc --target-dir target/xcheck`
  (the `llm` feature needs a C cross-compiler for `ring`).
- Static agent: `cargo build -p contextual-agent --release --target x86_64-unknown-linux-musl`.
- Checking flows without clicking (debug builds only, `crates/app/src/dev.rs`):
  `CONTEXTUAL_DEV_SEND="…" CONTEXTUAL_DEV_AFTER=diff|split|save|undo|chats|inspect`,
  `CONTEXTUAL_DEV_OPEN=scm|settings|branches|pr|inspect|gitdiff:<path>|commit:<msg>|switch:<branch>`
  (`envfile` with `--settings`: the env-file picker; `models` with `--settings`: the Models group),
  `CONTEXTUAL_DEV_REMOTE=1` (serve projects through a local agent: the WSL code path),
  `CONTEXTUAL_DEV_AFTER=command-run|command-stop|command-reject|command-edit|command-share|command-withhold|command-decline|command-quit|command-close`
  (simulate explicit command-card actions; no command-enablement variable is needed). Point
  `AZURE_OPENAI_*` at a local mock (`http://localhost:<port>/`) when no key is at hand; add
  `AZURE_OPENAI_ROUTER` and `AZURE_OPENAI_<NAME>_DEPLOYMENT` / `_TIER` per model to try routing.
- Test fixtures were recorded from the old TypeScript app; see `crates/core/tests/fixtures/README.md`.

## UI design
The UI spec is the Contextual design system in `docs/design/contextual/` (source in `SOURCE.md`).
Read it before building UI: `README.md` (principles, code rendering, layout), `tokens.json`,
`components/*/README.md` (one spec per component and screen).
- `tokens.json` is the **only** source of colours, spacing, radii, shadows and type styles. Views
  use them through `crates/app/src/theme.rs`: `hsla(cx.tokens().colors.<token>)`,
  `.type_style(type_style::UI_BODY)`, `sp(n)` (steps 1, 2, 3, 4, 6, 8), `row_height()`,
  `control_height()`, `radius::*`, `shadow(…)`, layout widths in `contextual_theme::layout`.
  Never write literal colours (CI rejects them in `crates/app/src`) or invent spacing.
- gpui-component's theme and the syntax highlight theme are set from the same tokens in
  `theme::apply`; code ligatures are off.
- Prefer small composable views and primitives in `ui/` over flag-heavy components.
- `components/bundle.js` / `bundle.css` in the design folder are reference code only.

## Project skills (`.claude/skills/`, tracked in `skills-lock.json`)
- `frontend-design` — visual direction and polish.
- `web-design-guidelines` — UI/accessibility review.
