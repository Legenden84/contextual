# contextual-core

Everything in Contextual except the UI. It replaces the old Node server (`src/server`, whose
routes map onto the functions below) and the pure logic of the old web app (`src/web/lib`).
Those sources are in git history at commit `b578d2c`.

## Workspace host

All project-file operations go through `host::WorkspaceHost` and run on the machine the project
lives on: in-process (`LocalHost`) or inside a WSL distro through `contextual-agent`
(`remote::RemoteHost`, supervised by `remote::SupervisedHost`). Calls are refused for roots the
user didn't open (`allow_root`).

| Call | Old route | What |
| --- | --- | --- |
| `list_dirs` | `GET /api/fs/dirs` | Sub-folders of a path, plus places (home, `/`, drives or volumes). |
| `scan` | `POST /api/project/scan` | Tree, token estimates, exclusion reasons (`ignored: <dir>`, `lock file`, `.gitignore`, `binary`, `too large`, `hidden`, `secrets`), images, condensed HTML/SVG, git branch, Skills & Rules (`SKILL.md` anywhere, 20,000-entry cap). AI-tool folders (`.claude`, `.codex`, `.cursor`, `.gemini`, `.deepseek`… see `AGENT_DIRS`) are left out of the tree but searched for skills and rules; `.claude/worktrees` is never searched. Framework presets (`logic::defaults::BUILTIN_PRESETS`: Rust, Node, .NET, Python, Go…, editable in Settings) add their globs when one of their markers (`Cargo.toml`, `package.json`, `*.csproj`…) is at the root or up to two folders down; the result lists the detected ones in `frameworks`. |
| `read` | `POST /api/project/read` | Contents (LF, no BOM, 2 MB cap), images as data URLs, condensed references. |
| `apply` | `POST /api/project/apply` | Save in place: every file checked against what the model saw first, nothing written if any check fails; line endings, BOM and final newline kept; atomic writes; undo snapshot (last 20). |
| `undo` | `POST /api/project/undo` | Restore a snapshot; refused if a saved file was edited since. |
| `git(GitCall::…)` | `/api/git/*`, `/api/pr/*` | Status (VS Code letters), stage/unstage, file diff, commit (smart commit), push/publish, branches, switch, create (parent recorded), PR compare, diff for drafts, create via `gh`. |

## Assistant-proposed commands (in development)

`commands` contains transport-independent request validation and an in-memory approval gate.
`WorkspaceHost::prepare_command` performs non-executing preparation on the local or remote
project host: native directory validation, executable/interpreter resolution, restricted
environment construction and host-instance identification. Remote preparation requires the
protocol-v3 `commandPrepare` capability and is not automatically retried.

`commands::execution` adds an execution supervisor with write-ahead journal requirements,
fresh preparation checks, bounded output/progress, cancellation and timeout handling.
It requires an explicit managed process-tree backend; the existing Git runner is not one.
Only fake backends are currently supplied for tests.

There is still no native approved-command runner or executable chat action.
See the [staged implementation plan](../../docs/plans/approved-command-execution.md).

Approval applies to one exact prepared invocation, including its project/host instance,
working directory, resolved executable, arguments and complete environment. It cannot be
serialized or reused after reopening. The existing Git runner is unchanged.

A working-directory containment check is not a subprocess sandbox. Approved commands may
access other files and the network; their side effects will not be covered by Save/Undo.

## App-side modules

- `llm` — Foundry client: Responses API (`store: true`, `previous_response_id`, chain-reset
  retry), fallback to chat completions, SSE streaming as `ChatEvent`s, cancellation, test
  connection, commit/PR drafts, edit repair. The API key never leaves the app process.
- `chat` — a turn: which files to attach, the saved-thread JSON, notes after a reply.
- `logic` — pure functions shared by the UI (diff, markdown, changes, edits, prompt, tree,
  context totals, images, PR helpers; `reads`: the model picks its files — the `read` protocol,
  the project tree and skill index it is sent, checking requests and the per-message limits;
  `route`: several models — the routing model's prompt, the `route` block it answers with and
  the tier rules applied to it).
- `threads` (saved chats, per project key; `import_legacy` copies the old app's), `settings`
  (`settings.json`, env overrides, `SecretStore`), `recents` (`state.json`), `migrate`
  (`.env.local` import), `models` (the user's model descriptions under `models` in `settings.json`: context
  limit, prices, tier and description; the app ships none), `location` and `wsl` (paths to
  locations, distros, agent deployment).

`llm`, `settings`, `models`, `chat` and `migrate` are behind the default `llm` feature; the agent builds
without it (no HTTP/TLS stack).
