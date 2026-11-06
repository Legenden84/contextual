# src/server

Node side of Contextual. There is no standalone backend: `plugin.ts` is a Vite plugin that
mounts these routes on the local dev/preview server (`npm run dev` / `npm run preview`).

| File | Role |
| --- | --- |
| `plugin.ts` | Loads `.env` / `.env.local` (server-only) and mounts `/api` |
| `config.ts` | Reads and validates the Azure AI Foundry variables |
| `foundry.ts` | Conversation turns: Responses API (`previous_response_id`) with fallback to chat completions; connection test |
| `threads.ts` | Saved conversations per project (`~/.cache/contextual/threads/`) |
| `git.ts` | Pull requests: repo info, diff of the applied files, commit only those files, push, `gh pr create` |
| `api.ts` | Routes, body parsing, same-origin check |
| `fs/paths.ts` | Path normalising (`C:\x` → `/mnt/c/x`, `\\wsl$\Ubuntu\…` → `/…`, `~`) and keeping paths inside the project |
| `fs/browse.ts` | Folder listing for the Open folder dialog |
| `fs/scan.ts` | Project scan: tree, token estimates, exclusions, git branch, instruction files |
| `fs/condense.ts` | Condensed copies of large HTML/SVG (embedded data and minified code replaced by placeholders) |
| `fs/images.ts` | Image formats sent to the model as pictures |
| `fs/files.ts` | Reading files; saving applied changes (editor-style) and Undo |

## Routes

- `GET /api/health` → `{ configured, endpoint, deployment, modelName, … }` or `{ configured: false, missing, invalid }`. Never returns the key.
- `POST /api/test` → `{ ok, latencyMs }` (a 16-token round trip).
- `POST /api/chat` with `{ system, history, input, previousResponseId }` → `application/x-ndjson`, one event
  per line: `delta` (text), `usage` (with `cachedTokens`), `done` (`responseId`, `api`, `chainReset`), or `error`.
  `input` (and user turns in `history`) is a string or `[{ type: "text", … }, { type: "image_url", image_url: { url: "data:image/png;base64,…" } }]`.
  - **Responses API first:** on Azure's v1 endpoint (`<endpoint>/openai/v1/responses`, `store: true`), sending
    `system` as `instructions` and only `input` plus `previous_response_id`. Azure keeps the conversation (~30 days).
  - If the stored chain is gone, it retries once with the full `history` and reports `chainReset`.
  - If the deployment has no Responses API (404 / unsupported), it uses chat completions with
    `[system, ...history, input]` from then on. `history` is kept byte-for-byte stable by the client, so
    Azure's prompt cache applies.
- `GET /api/threads?root=` · `GET /api/threads/<id>?root=` · `POST /api/threads/save { root, id, summary, thread }` ·
  `POST /api/threads/delete { root, id }` → saved conversations (last 30 per project, outside the project).

- `GET /api/fs/dirs?path=&hidden=1` → sub-folders of a path, plus Home, `/` and Windows drives.
- `POST /api/project/scan` `{ root, options }` → tree, rules, branch. Exclusion reasons: `ignored: <dir>`,
  `lock file`, `.gitignore` (root `.gitignore` only), `binary`, `too large`, `hidden`, `secrets` (`.env*`, always).
  Images are marked `image` (~765 tokens each). Large HTML/SVG get `condensedFrom`, and `tokens` and the size
  limit use the condensed size. "too large" files can still be ticked one by one in the UI.
- `POST /api/project/read` `{ root, paths, condense? }` → contents (text up to 2 MB each, LF line endings,
  no BOM). PNG/JPEG/GIF/WebP files come back as `image` data URLs (up to 20 MB). With `condense`, HTML/SVG
  of 32 KB or more come back condensed, with `condensedFrom` set; the client sends those as read-only
  references and never offers changes to them.
- `POST /api/project/apply` `{ root, changes, backup }` → saves the changes in place in the working tree,
  like Save in an editor: nothing is staged or committed. Each file keeps its line endings (CRLF/LF),
  BOM and final-newline setting, so `git diff` shows only the real edits. Every file is checked first
  (a modified/deleted file must still match what the model saw; a new file must not exist) and nothing
  is saved if any check fails. With `backup`, originals go to an undo snapshot outside the project
  (`~/.cache/contextual/undo/<project hash>/<id>/`, last 20 kept).
- `POST /api/project/undo` `{ root, backupId }` → restores that snapshot and removes files and folders
  the save created. Refused if any of those files was edited after the save.

- `POST /api/git/info` `{ root, paths }` → branch, default branch, `origin`, whether `gh` can reach the repo,
  each path's status vs HEAD, and an open PR for the current branch.
- `POST /api/pr/draft` `{ root, paths, notes }` → `{ title, body }` written by the model from the chat notes and
  `git diff HEAD` of the paths (one non-streaming request, not part of the conversation).
- `POST /api/pr/create` `{ root, paths, title, body, base, draft, newBranch? }` → stages and commits **only** `paths`
  (`git commit --only`; other staged/unstaged changes are untouched) on the current branch, or on `newBranch`
  (required when on the default branch; switched back and deleted if the commit fails), pushes with
  `git push -u origin HEAD`, then `gh pr create` (or just pushes if the branch already has an open PR).
  Needs the GitHub CLI logged in (`gh auth login`); git never prompts for credentials.

Requests must be `Content-Type: application/json` and same-origin, so other websites can't use
the local server to spend tokens.

## Configuration

Copy `.env.example` to `.env.local` and set `AZURE_OPENAI_API_KEY`. The endpoint must be HTTPS,
except `http://localhost` for a local mock or proxy. Restart `npm run dev` after editing it.
Never import from `src/web` here, and never import this folder from `src/web`.
