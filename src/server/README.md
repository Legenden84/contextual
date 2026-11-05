# src/server

Node side of Contextual. There is no standalone backend: `plugin.ts` is a Vite plugin that
mounts these routes on the local dev/preview server (`npm run dev` / `npm run preview`).

| File | Role |
| --- | --- |
| `plugin.ts` | Loads `.env` / `.env.local` (server-only) and mounts `/api` |
| `config.ts` | Reads and validates the Azure AI Foundry variables |
| `foundry.ts` | `AzureOpenAI` client from the `openai` SDK, streaming chat, connection test |
| `api.ts` | Routes, body parsing, same-origin check |

## Routes

- `GET /api/health` → `{ configured, endpoint, deployment, modelName, … }` or `{ configured: false, missing, invalid }`. Never returns the key.
- `POST /api/test` → `{ ok, latencyMs }` (a 16-token round trip).
- `POST /api/chat` with `{ "messages": [{ "role": "system" | "user" | "assistant", "content": "…" }] }`
  → `application/x-ndjson`, one event per line: `delta` (text), `usage`, `done`, or `error`.

Requests must be `Content-Type: application/json` and same-origin, so other websites can't use
the local server to spend tokens.

## Configuration

Copy `.env.example` to `.env.local` and set `AZURE_OPENAI_API_KEY`. The endpoint must be HTTPS,
except `http://localhost` for a local mock or proxy. Restart `npm run dev` after editing it.
Never import from `src/web` here, and never import this folder from `src/web`.
