//! Defaults from `src/web/data/defaults.ts`: the output-format system prompt, exclusion globs,
//! and fixed token estimates. The prompts are stored already dedented.

use super::types::Deployment;

/// System prompt for the model's behaviour (editable in Settings).
pub const OUTPUT_PROMPT: &str =
    "You are helping edit files in a local project. The user reviews every change
and saves it manually. You cannot write to disk.

- Explain the change in a few sentences first.
- Only touch files that are included in the context.
- Follow the project rules provided above this prompt.";

/// Earlier defaults of [`OUTPUT_PROMPT`]. A saved setting equal to one of these was never edited
/// by the user, so it is replaced by the current default.
pub const LEGACY_OUTPUT_PROMPTS: &[&str] = &[
    "You are helping edit files in a local project. The user reviews every change
and applies it manually. You cannot write to disk.

## Output format
- Explain the change in a few sentences first.
- For every file you add or change, return its COMPLETE new contents.
  Never return partial snippets, \"...\" or \"unchanged\" placeholders.
- Put each file in its own fenced block, directly after a header line:

### FILE: src/Path/Relative/To/Root.cs
```csharp
<full file contents>
```

- To delete a file, write a single line: ### DELETE: src/Path/To/File.cs
- Only touch files that are included in the context.
- Follow the project rules provided above this prompt.",
];

/// Default exclusion globs (editable in Settings).
pub const DEFAULT_EXCLUSIONS: &[&str] = &[
    "**/node_modules/**",
    "**/.venv/**",
    "**/venv/**",
    "**/__pycache__/**",
    "**/bin/**",
    "**/obj/**",
    "**/dist/**",
    "**/build/**",
    "**/.git/**",
    "package-lock.json",
    "yarn.lock",
    "pnpm-lock.yaml",
    "poetry.lock",
    "Pipfile.lock",
    "packages.lock.json",
    "Cargo.lock",
];

/// Shown until the local server reports the deployment from the configuration.
pub fn placeholder_deployment() -> Deployment {
    Deployment {
        name: "not configured".into(),
        limit: 128_000,
        in_price: 0.0,
        out_price: 0.0,
    }
}

/// Estimated tokens of a typical user message, for the context bar.
pub const USER_MESSAGE_TOKENS: u64 = 96;
/// Output tokens assumed for the per-message cost estimate.
pub const EST_OUTPUT_TOKENS: u64 = 1800;
