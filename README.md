# Contextual

Contextual is a native Rust desktop chat client for working with project files through
Azure AI Foundry. Select files as context, review proposed changes, and explicitly save or
undo them. The model never writes to disk.

The desktop targets are **Windows and macOS**. Day-to-day development currently happens in
**WSL (Ubuntu)**, where the Linux app opens through WSLg; Linux is not a supported desktop
distribution target. Native Windows/macOS validation is still in progress—see the
[port plan](docs/plans/cross-platform-rust-port.md) for current status.

## Getting started

1. Follow either [macOS setup](#macos-setup) or [WSL setup](#wsl-setup).
2. Clone your fork or the project repository and open a terminal in its root.
3. Run the [build and test commands](#build-run-and-test).
4. Configure Azure AI Foundry only if you want to use live model features.

You do **not** need an API key to build the app or run the automated tests. No Node.js or
JavaScript package manager is required.

Use the latest stable Rust toolchain. The workspace declares Rust 1.85 as its minimum
(edition 2024), but dependencies may require a newer compiler.

## macOS setup

These instructions cover native development on Apple Silicon and Intel Macs. The project's
development loop has been validated on WSL; this macOS checklist still needs verification
on a clean machine.

### 1. Install Apple's build tools

Install **Xcode** from the Mac App Store and open it once to finish installation.
Use full Xcode rather than only the standalone Command Line Tools: GPUI's macOS renderer
uses Metal and its build tools.

Select the installed Xcode and complete its first-launch setup:

```bash
sudo xcode-select --switch /Applications/Xcode.app/Contents/Developer
sudo xcodebuild -runFirstLaunch
```

If you installed Xcode elsewhere, adjust the path. If a build reports missing Metal tools,
install the Metal Toolchain component through Xcode's settings.

### 2. Install command-line dependencies

Install [Homebrew](https://brew.sh/) if needed, and follow its instructions to add `brew`
to your shell's `PATH`. Then install:

```bash
brew install pkg-config git
```

GitHub CLI is optional; install it for the app's pull-request features or to manage PRs
from your terminal:

```bash
brew install gh
gh auth login
```

The Linux GUI packages listed in the WSL section are not needed on macOS.

### 3. Install Rust

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
source "$HOME/.cargo/env"
rustup update stable
rustup default stable
rustup component add rustfmt clippy rust-analyzer
```

Rustup selects your native target: `aarch64-apple-darwin` on Apple Silicon or
`x86_64-apple-darwin` on Intel. No cross-compilation targets are needed for a local build.

Verify the tools, then continue to [Build, run, and test](#build-run-and-test):

```bash
rustc --version
cargo --version
git --version
xcode-select --print-path
```

## WSL setup

Use **WSL 2 with Ubuntu** and WSLg support on Windows. Run the Linux commands below inside
your Ubuntu terminal, not PowerShell.

### 1. Install or update WSL

If WSL is not installed, run this in an administrator PowerShell terminal and follow the
prompts, including any restart:

```powershell
wsl --install -d Ubuntu
```

For an existing installation, update WSL and check that your distro uses version 2:

```powershell
wsl --update
wsl --list --verbose
```

Keep your checkout in the Linux filesystem, such as `~/src/contextual`, rather than under
`/mnt/c`, for better build and file-watching performance.

### 2. Install system dependencies

Inside Ubuntu:

```bash
sudo apt-get update
sudo apt-get install -y \
  build-essential pkg-config clang curl ca-certificates git \
  libssl-dev libxkbcommon-dev libxkbcommon-x11-dev \
  libwayland-dev libx11-xcb-dev libxcb1-dev \
  libfontconfig-dev libfreetype-dev libvulkan-dev \
  mesa-vulkan-drivers libasound2-dev
```

These provide:

- **Compilers and build tools:** needed by Rust crates that compile C/C++ code.
- **GUI development libraries:** GPUI's Linux windowing, text, graphics, and audio dependencies.
- **Mesa Vulkan drivers:** support rendering the development window through WSLg.

GitHub CLI is optional; install and authenticate it if you need pull-request features:

```bash
sudo apt-get install -y gh
gh auth login
```

For projects opened through the Windows app's WSL host, Git and `gh` run inside the distro,
so configure credentials there too.

### 3. Install Rust

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
source "$HOME/.cargo/env"
rustup update stable
rustup default stable
rustup component add rustfmt clippy rust-analyzer
```

Verify the tools, then continue to [Build, run, and test](#build-run-and-test):

```bash
rustc --version
cargo --version
git --version
```

### 4. Optional: Windows and agent cross-compilation

These tools are **not required** to build and run the Linux app through WSLg.
The following commands are for an x86_64 Ubuntu development environment.

Install MinGW and the additional Rust targets:

```bash
sudo apt-get install -y gcc-mingw-w64-x86-64 g++-mingw-w64-x86-64
rustup target add \
  x86_64-pc-windows-gnu \
  x86_64-unknown-linux-musl \
  x86_64-pc-windows-msvc
```

From the repository root, build the Windows debug executable:

```bash
cargo build -p contextual --target x86_64-pc-windows-gnu
```

Launch `target/x86_64-pc-windows-gnu/debug/contextual.exe` from Windows to test the native
Windows UI rather than the Linux UI through WSLg. Windows Smart App Control may block
unsigned local builds. Windows release builds require additional shader tooling and are
not covered by these instructions.

Build the static WSL agent:

```bash
cargo build -p contextual-agent --release --target x86_64-unknown-linux-musl
```

The agent builds without core's HTTP/TLS feature and currently needs no separate musl C
toolchain. Building the agent alone does not package it with the Windows executable; see
the [port plan](docs/plans/cross-platform-rust-port.md) for deployment details.

Optionally type-check the core for Windows without linking a Windows executable:

```bash
cargo clippy -p contextual-core --no-default-features \
  --target x86_64-pc-windows-msvc --target-dir target/xcheck
```

This deliberately disables the `llm` feature, whose TLS dependencies need a C cross-compiler.

## Build, run, and test

Run these commands from the cloned repository's root.

Build and launch the desktop app:

```bash
cargo run -p contextual
```

Open a particular project or start without restoring the previous project:

```bash
cargo run -p contextual -- /path/to/project
cargo run -p contextual -- --no-restore
```

Run the full contributor checks before opening a pull request:

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Plain `cargo build` and `cargo test` use the default workspace members, which **exclude the
desktop app on every platform**. They are useful for headless work, but do not verify UI
changes. Use `--workspace` to include everything, or `-p contextual` to select the app.

For editor support, install your editor's Rust integration (for example, the rust-analyzer
extension in VS Code). With VS Code on Windows, open the checkout through the WSL extension
so the editor uses the Linux Rust toolchain.

## Azure AI Foundry configuration

For live chat and model-assisted features, launch Settings:

```bash
cargo run -p contextual -- --settings
```

Enter your endpoint, deployment, API version, and API key, then test the connection.
Settings also supports importing an existing `.env.local` file.

The app uses the OS keychain when available. In WSL, a keychain service is often unavailable;
the app then keeps a key entered in Settings only for the current session.

For development, you can provide `AZURE_OPENAI_API_KEY` through your shell environment or a
secret manager before launching the app. Environment variables override saved Foundry
configuration. Avoid putting real keys in shell-history commands, committed files,
screenshots, or PRs.

## Contributing

- Read [CLAUDE.md](CLAUDE.md) for workspace conventions and development commands.
- Keep business rules in `crates/core`; see its [README](crates/core/README.md).
- For UI changes, read the [design system](docs/design/contextual/README.md).
  Colours, spacing, and typography come from its tokens, not hard-coded values.
- Add or update tests for changed behaviour and run the full checks above.
- In your PR, describe the change and which platforms you tested. A successful WSLg run
  does not replace native Windows or macOS testing.

Before switching machines, commit and push work you want to carry over; uncommitted
changes remain on the old machine.

## Troubleshooting

### Rust or Cargo is not found

Open a new terminal or run:

```bash
source "$HOME/.cargo/env"
```

Rustup installs its tools under `~/.cargo/bin`. If a dependency requires a newer compiler,
run `rustup update stable`.

### The window does not open in WSL

Confirm the distro uses WSL 2 and that WSLg is available. Run `wsl --update` from Windows
PowerShell and check your Windows graphics drivers.

Some libEGL/Mesa warnings can appear even when the app works. If no window appears or
rendering fails, do not assume the warnings are harmless; include them in your bug report.

### Image drag-and-drop in WSLg

Drop PNG, JPEG, GIF, or WebP files anywhere in the chat pane. They are attached to the draft,
not sent until you press Send.

The Linux app translates Windows attachment paths with `wslpath` when running in WSL.
However, WSLg does not reliably forward file drags from Windows Explorer to Linux windows.
Path translation cannot help if no drop event reaches the app. In that case:

- Use **Attach images…** and browse to the Windows file under `/mnt/c/Users/...`
  (or your configured drive mount).
- For native Explorer drag-and-drop, build and launch the Windows executable as described
  above.

When changing attachment handling, test multiple images over both an existing message and
the composer; each should appear once and be removable. Also try an unsupported file, an
unreadable file, and an image over 20 MiB. Dropping while a response is running should report
that you must wait.

Test native Windows Explorer and macOS Finder separately from WSLg; a successful
path-conversion unit test does not verify desktop drag-event forwarding.