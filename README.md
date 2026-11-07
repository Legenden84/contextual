# Contextual: WSL setup for Rust development

These steps match the current dev machine (Ubuntu 26.04 in WSL, Rust stable 1.99) plus what CI installs. Run them inside the WSL distro.

Before switching machines, commit and push any local work. Uncommitted changes stay behind on the old machine.

## 1. System packages

sudo apt-get update && sudo apt-get install -y build-essential pkg-config clang git gh libssl-dev libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev libx11-xcb-dev libxcb1-dev libfontconfig-dev libfreetype-dev libvulkan-dev mesa-vulkan-drivers libasound2-dev gcc-mingw-w64-x86-64 g++-mingw-w64-x86-64
What they're for:

**Compilers and build tools** (build-essential, pkg-config, clang): building crates with C code, such as ring.
**GUI libraries for GPUI** (the libx*, wayland, fontconfig, freetype, vulkan and asound packages): the same list CI uses.
**mesa-vulkan-drivers:** lets the app draw its window through WSLg.
***-mingw-w64-x86-64:** cross-compiling contextual.exe for Windows.
## 2. Rust (rustup)

The project needs Rust 1.85 or newer (edition 2024). Plain stable is fine.

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source "$HOME/.cargo/env"
rustup component add rustfmt clippy rust-analyzer
```

## 3. Cross-compile targets

rustup target add x86_64-pc-windows-gnu x86_64-unknown-linux-musl x86_64-pc-windows-msvc
**x86_64-pc-windows-gnu:** builds the Windows .exe (uses mingw from step 1).
**x86_64-unknown-linux-musl:** builds the static WSL agent. It's pure Rust, so it needs no musl C toolchain.
**x86_64-pc-windows-msvc:** only for the Windows type-check of contextual-core with clippy.
**Optional:** aarch64-apple-darwin was installed on the old machine, but nothing in CLAUDE.md uses it.
## 4. GitHub login (for PRs from the app and gh)

gh auth login
## 5. Check the setup

Run these in the cloned repo:

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
cargo build -p contextual --target x86_64-pc-windows-gnu
cargo build -p contextual-agent --release --target x86_64-unknown-linux-musl
cargo run -p contextual
```

## 6. API key

WSL has no keychain, so the app keeps the API key in memory only and loses it every time the app quits. Set it in your shell profile instead:

echo "export AZURE_OPENAI_API_KEY='...'" >> ~/.bashrc
The other values (endpoint, deployment, API version) can go in ~/.bashrc the same way, or you can enter them in Settings. Settings saves them in ~/.config/contextual/settings.json.

## Image drag-and-drop during development

Drop PNG, JPEG, GIF or WebP files anywhere in the chat pane. They are attached to the draft,
not sent until you press Send.

The Linux app translates Windows attachment paths with `wslpath` when running in WSL.
However, WSLg does not reliably forward file drags from Windows Explorer to Linux windows.
Path translation cannot help if no drop event reaches the app. In that case:

- Use **Attach images…** and browse to the Windows file under `/mnt/c/Users/...`
  (or your configured drive mount).
- For native Explorer drag-and-drop, build the Windows executable using the cross-compile
  command above and launch `target/x86_64-pc-windows-gnu/debug/contextual.exe` from Windows.
  That runs the Windows UI rather than the Linux UI through WSLg.

Manual checks: drop multiple images over both an existing message and the composer; check that
each appears once and can be removed. Try an unsupported file, an unreadable file and an image
over 20 MiB. Verify that dropping while a response is running reports that you must wait.
Test native Windows Explorer and macOS Finder separately from WSLg; a successful path-conversion
unit test does not verify desktop drag-event forwarding.

## Notes

**If the window doesn't open:** WSLg must be enabled. Make sure WSL is up to date by running wsl --update in Windows PowerShell.
**Expected warnings:** libEGL / MESA warnings in the terminal are normal on WSLg, and the app still runs.