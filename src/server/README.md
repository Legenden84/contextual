# src/server

Node side of Contextual. There is no standalone backend: `plugin.ts` is a Vite plugin that
mounts these routes on the local dev/preview server (`npm run dev` / `npm run preview`).

| File | Role |
| --- | --- |
| `plugin.ts` | Loads `.env` / `.env.local` (server-only) and mounts `/api` |
| `config.ts` | Reads and validates the Azure AI Foundry variables |
| `foundry.ts` | `AzureOpenAI` client from the `openai` SDK, streaming chat, connection test |
| `api.ts` | Routes, body parsing, same-origin check |
| `fs/paths.ts` | Path normalising (`C:\x` → `/mnt/c/x`, `\\wsl$\Ubuntu\…` → `/…`, `~`) and keeping paths inside the project |
| `fs/browse.ts` | Folder listing for the Open folder dialog |
| `fs/scan.ts` | Project scan: tree, token estimates, exclusions, git branch, instruction files |
| `fs/files.ts` | Reading files; Apply (with backup) and Undo |

## Routes

- `GET /api/health` → `{ configured, endpoint, deployment, modelName, … }` or `{ configured: false, missing, invalid }`. Never returns the key.
- `POST /api/test` → `{ ok, latencyMs }` (a 16-token round trip).
- `POST /api/chat` with `{ "messages": [{ "role": "system" | "user" | "assistant", "content": "…" }] }`
  → `application/x-ndjson`, one event per line: `delta` (text), `usage`, `done`, or `error`.

- `GET /api/fs/dirs?path=&hidden=1` → sub-folders of a path, plus Home, `/` and Windows drives.
- `POST /api/project/scan` `{ root, options }` → tree, rules, branch. Exclusion reasons: `ignored: <dir>`,
  `lock file`, `.gitignore` (root `.gitignore` only), `binary`, `too large`, `hidden`, `secrets` (`.env*`, always).
- `POST /api/project/read` `{ root, paths }` → contents (text only, up to 2 MB each).
- `POST /api/project/apply` `{ root, changes, backup }` → checks every file first (a modified/deleted file must
  still match what the model saw; a new file must not exist) and writes nothing if any check fails. With
  `backup`, originals go to `.contextual/backups/<id>/` (git-ignored by its own `.gitignore`).
- `POST /api/project/undo` `{ root, backupId }` → restores that backup.

Requests must be `Content-Type: application/json` and same-origin, so other websites can't use
the local server to spend tokens.

## Configuration

Copy `.env.example` to `.env.local` and set `AZURE_OPENAI_API_KEY`. The endpoint must be HTTPS,
except `http://localhost` for a local mock or proxy. Restart `npm run dev` after editing it.
Never import from `src/web` here, and never import this folder from `src/web`.
