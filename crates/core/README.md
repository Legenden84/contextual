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
| `scan` | `POST /api/project/scan` | Tree, token estimates, exclusion reasons (`ignored: <dir>`, `lock file`, `.gitignore`, `binary`, `too large`, `hidden`, `secrets`), images, condensed HTML/SVG, git branch, Skills & Rules (`SKILL.md` anywhere, 20,000-entry cap). AI-tool folders (`.claude`, `.codex`, `.cursor`, `.gemini`, `.deepseek`… see `AGENT_DIRS`) are left out of the tree but searched for skills and rules; `.claude/worktrees` is never searched. |
| `read` | `POST /api/project/read` | Contents (LF, no BOM, 2 MB cap), images as data URLs, condensed references. |
| `apply` | `POST /api/project/apply` | Save in place: every file checked against what the model saw first, nothing written if any check fails; line endings, BOM and final newline kept; atomic writes; undo snapshot (last 20). |
| `undo` | `POST /api/project/undo` | Restore a snapshot; refused if a saved file was edited since. |
| `git(GitCall::…)` | `/api/git/*`, `/api/pr/*` | Status (VS Code letters), stage/unstage, file diff, commit (smart commit), push/publish, branches, switch, create (parent recorded), PR compare, diff for drafts, create via `gh`. |

## App-side modules

- `llm` — Foundry client: Responses API (`store: true`, `previous_response_id`, chain-reset
  retry), fallback to chat completions, SSE streaming as `ChatEvent`s, cancellation, test
  connection, commit/PR drafts, edit repair. The API key never leaves the app process.
- `chat` — a turn: which files to attach, the saved-thread JSON, notes after a reply.
- `logic` — pure functions shared by the UI (diff, markdown, changes, edits, prompt, tree,
  context totals, images, PR helpers).
- `threads` (saved chats, per project key; `import_legacy` copies the old app's), `settings`
  (`settings.json`, env overrides, `SecretStore`), `recents` (`state.json`), `migrate`
  (`.env.local` import), `location` and `wsl` (paths to locations, distros, agent deployment).

`llm`, `settings`, `chat` and `migrate` are behind the default `llm` feature; the agent builds
without it (no HTTP/TLS stack).
