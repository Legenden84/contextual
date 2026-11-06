//! Defaults from `src/web/data/defaults.ts`: the output-format system prompt, exclusion globs,
//! and fixed token estimates. The prompts are stored already dedented.

use contextual_protocol::fs::ExclusionPreset;

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
/// The old app's single exclusion list (`defaultExclusions`). A saved list equal to this was
/// never edited and is upgraded to [`DEFAULT_EXCLUSIONS`] plus [`default_presets`].
pub const LEGACY_EXCLUSIONS: &[&str] = &[
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

/// Exclusions for every project. Build output and generated files differ per ecosystem and are
/// in the framework presets instead.
pub const DEFAULT_EXCLUSIONS: &[&str] = &[
    "**/.git/**",
    "package-lock.json",
    "yarn.lock",
    "pnpm-lock.yaml",
    "bun.lockb",
    "bun.lock",
    "poetry.lock",
    "Pipfile.lock",
    "uv.lock",
    "packages.lock.json",
    "Cargo.lock",
    "composer.lock",
    "Gemfile.lock",
    "go.sum",
    "pubspec.lock",
    "Package.resolved",
    "**/*.min.js",
    "**/*.min.css",
    "**/*.js.map",
    "**/*.css.map",
];

/// A built-in framework preset (see [`default_presets`]).
pub struct BuiltinPreset {
    pub id: &'static str,
    pub label: &'static str,
    pub markers: &'static [&'static str],
    pub exclusions: &'static [&'static str],
}

/// Framework presets: build output, dependencies, caches, generated code and test data of each
/// ecosystem, applied only to projects that contain one of the markers.
pub const BUILTIN_PRESETS: &[BuiltinPreset] = &[
    BuiltinPreset {
        id: "rust",
        label: "Rust",
        markers: &["Cargo.toml"],
        exclusions: &[
            "**/target/**",
            "**/tests/fixtures/**",
            "**/snapshots/*.snap",
        ],
    },
    BuiltinPreset {
        id: "node",
        label: "Node / TypeScript",
        markers: &["package.json", "deno.json", "deno.jsonc"],
        exclusions: &[
            "**/node_modules/**",
            "**/dist/**",
            "**/build/**",
            "**/out/**",
            "**/coverage/**",
            "**/.next/**",
            "**/.nuxt/**",
            "**/.svelte-kit/**",
            "**/.turbo/**",
            "**/.parcel-cache/**",
            "**/storybook-static/**",
            "**/__snapshots__/**",
            "**/__fixtures__/**",
            "*.tsbuildinfo",
        ],
    },
    BuiltinPreset {
        id: "dotnet",
        label: ".NET",
        markers: &[
            "*.sln",
            "*.slnx",
            "*.csproj",
            "*.fsproj",
            "*.vbproj",
            "global.json",
        ],
        exclusions: &[
            "**/bin/**",
            "**/obj/**",
            "**/TestResults/**",
            "**/packages/**",
        ],
    },
    BuiltinPreset {
        id: "python",
        label: "Python",
        markers: &[
            "pyproject.toml",
            "setup.py",
            "setup.cfg",
            "requirements*.txt",
            "Pipfile",
        ],
        exclusions: &[
            "**/__pycache__/**",
            "**/.venv/**",
            "**/venv/**",
            "**/*.egg-info/**",
            "**/.pytest_cache/**",
            "**/.mypy_cache/**",
            "**/.ruff_cache/**",
            "**/.tox/**",
            "**/htmlcov/**",
            "**/dist/**",
            "**/build/**",
        ],
    },
    BuiltinPreset {
        id: "go",
        label: "Go",
        markers: &["go.mod", "go.work"],
        exclusions: &["**/vendor/**", "**/testdata/**"],
    },
    BuiltinPreset {
        id: "jvm",
        label: "Java / Kotlin",
        markers: &[
            "pom.xml",
            "build.gradle",
            "build.gradle.kts",
            "settings.gradle",
            "settings.gradle.kts",
        ],
        exclusions: &["**/target/**", "**/build/**", "**/.gradle/**", "**/out/**"],
    },
    BuiltinPreset {
        id: "php",
        label: "PHP",
        markers: &["composer.json"],
        exclusions: &[
            "**/vendor/**",
            "**/storage/framework/**",
            "**/bootstrap/cache/**",
        ],
    },
    BuiltinPreset {
        id: "ruby",
        label: "Ruby",
        markers: &["Gemfile"],
        exclusions: &[
            "**/vendor/bundle/**",
            "**/.bundle/**",
            "**/tmp/**",
            "**/log/**",
        ],
    },
    BuiltinPreset {
        id: "dart",
        label: "Dart / Flutter",
        markers: &["pubspec.yaml"],
        exclusions: &[
            "**/.dart_tool/**",
            "**/build/**",
            "**/*.g.dart",
            "**/*.freezed.dart",
        ],
    },
    BuiltinPreset {
        id: "swift",
        label: "Swift / Xcode",
        markers: &["Package.swift", "*.xcodeproj", "*.xcworkspace", "Podfile"],
        exclusions: &["**/.build/**", "**/DerivedData/**", "**/Pods/**"],
    },
    BuiltinPreset {
        id: "cpp",
        label: "C / C++",
        markers: &[
            "CMakeLists.txt",
            "meson.build",
            "Makefile",
            "vcpkg.json",
            "conanfile.txt",
        ],
        exclusions: &[
            "**/build/**",
            "**/cmake-build-*/**",
            "**/out/**",
            "**/vcpkg_installed/**",
        ],
    },
];

impl BuiltinPreset {
    pub fn to_preset(&self) -> ExclusionPreset {
        let owned = |v: &[&str]| v.iter().map(|s| s.to_string()).collect();
        ExclusionPreset {
            id: self.id.to_string(),
            label: self.label.to_string(),
            enabled: true,
            markers: owned(self.markers),
            exclusions: owned(self.exclusions),
        }
    }
}

/// The built-in presets as editable settings.
pub fn default_presets() -> Vec<ExclusionPreset> {
    BUILTIN_PRESETS
        .iter()
        .map(BuiltinPreset::to_preset)
        .collect()
}

/// The built-in preset with this id.
pub fn builtin_preset(id: &str) -> Option<&'static BuiltinPreset> {
    BUILTIN_PRESETS.iter().find(|p| p.id == id)
}

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
