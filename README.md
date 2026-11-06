# Contextual

A local desktop tool for working on code with a self-hosted model (Azure AI Foundry): tick the
files the model should see, ask for a change, review the proposed diff file by file, and save what
you want. The model never writes to disk; saving is an explicit action, checked against what the
model saw, atomic and undoable. Source Control and pull requests (`git`, `gh`) are built in.

Native Rust app (GPUI) for **Windows and macOS**. On Windows it also opens projects that live
inside WSL, served by a small agent it deploys into the distro.

## Build and run

Development happens on Linux (WSL is fine); the app runs there too, through WSLg.

```sh
# one-time, for the GUI on Linux and the Windows cross-build
sudo apt-get install -y libxkbcommon-x11-dev libwayland-dev libx11-xcb-dev libfontconfig-dev \
  libfreetype-dev libvulkan-dev libasound2-dev clang mingw-w64

cargo run -p contextual                 # opens the last project
cargo run -p contextual -- ~/src/app    # opens a folder
cargo test --workspace
```

- Windows `.exe` from Linux (debug, for checks): `cargo build -p contextual --target x86_64-pc-windows-gnu`.
- Release builds for Windows and macOS come from CI (`.github/workflows/rust.yml`); the Windows
  artifact ships the static Linux agents (`contextual-agent-<arch>-linux`) next to `contextual.exe`.

## How the model gets context

With **Model picks files** (on by default, in the composer) a new chat starts with the project
tree (paths and token sizes; large folders folded) and an index of the skills. The model asks for
the files, folder listings and skills it needs with a `read` block; the app sends them and ticks
the files (tagged "model" in the explorer), up to 25 files / 150k tokens and 5 rounds per
message, asking before going further. Rules are always sent; skills are Auto (the model loads
them), Always or Never in Skills & Rules. A reply that changes a file the model wasn't sent is
answered with that file and asked again. Ticking files still sends them as before.

## Configuration

Settings → Azure AI Foundry: endpoint, deployment, API version and model name. The API key is
stored in the OS keychain (Credential Manager, Keychain, Secret Service), never in
`settings.json`. "Import .env.local" reads the old app's file.

Context limit and prices come from the model catalog, looked up by model name (else deployment
name): `crates/core/data/models.json` ships with the app; add a model there, or locally under
`"models"` in `settings.json` (same fields; the same `id` replaces a built-in entry). The
Settings fields and environment variables override the catalog for one deployment.

```json
{ "id": "gpt-6-astra", "label": "GPT-6 Astra", "aliases": [], "contextLimit": 922000,
  "inputPricePer1m": 10, "cachedInputPricePer1m": 1, "outputPricePer1m": 50,
  "longContext": { "aboveInputTokens": 272000, "inputPricePer1m": 20, "outputPricePer1m": 75 },
  "source": "https://…" }
```

These environment variables override the settings (handy for development):
`AZURE_OPENAI_ENDPOINT`, `AZURE_OPENAI_API_KEY`, `OPENAI_API_VERSION`, `AZURE_OPENAI_DEPLOYMENT`,
`AZURE_OPENAI_MODEL_NAME`, `AZURE_OPENAI_MAX_COMPLETION_TOKENS`, `AZURE_OPENAI_CONTEXT_LIMIT`,
`AZURE_OPENAI_INPUT_PRICE_PER_1M`, `AZURE_OPENAI_OUTPUT_PRICE_PER_1M`. The endpoint must be HTTPS,
or `http://localhost` for a local mock.

Data lives in the OS config/data dirs: `settings.json`, `state.json` (recent projects),
`threads/` (saved chats); undo snapshots in the cache dir of the machine the project is on.
Chats saved by the old app are copied in the first time their project is opened.

## Layout

| Crate | What |
| --- | --- |
| `crates/core` | Everything but the UI: workspace (scan, read, save/undo, git, PRs), Foundry client, chat turns, threads, settings, WSL deployment. See its README. |
| `crates/protocol` | Wire types shared by the app, the core and the agent (JSON-RPC). |
| `crates/theme` | The design tokens, generated at build time from `docs/design/contextual/tokens.json`. |
| `crates/agent` | `contextual-agent`: serves the workspace host over stdio JSON-RPC (inside WSL). |
| `crates/app` | `contextual`: the GPUI app. |

Design spec: `docs/design/contextual/`. Architecture and history:
`docs/plans/cross-platform-rust-port.md`.
