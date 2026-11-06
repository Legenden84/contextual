# Contextual

Local-only desktop-style developer tool: a chat client that sends selected project files
to a self-hosted LLM (Azure AI Foundry) and lets the user review and manually apply
proposed changes. The model never writes to disk; applying is an explicit user action.

## Stack
- React 19 + TypeScript + Vite 8, Tailwind CSS v4 (via `@tailwindcss/vite`, CSS-first config
  in `src/web/index.css` — there is no `tailwind.config.js`).
- Lint: `npm run lint` (oxlint). Build/typecheck: `npm run build`.

## Layout
- `src/web/` — the React app (browser). Path alias `@/` → `src/web/`.
  - `components/ui/` — small, generic primitives (Button, Checkbox, Switch, Segmented, Tag, Icon…).
  - `components/{code,diff,explorer,chat,changes,apply,shell}/` — app components built from them.
  - `screens/` — Welcome, Workspace, Diff, Settings.
  - `lib/` — pure logic: `syntax.ts` (highlighter), `diff.ts` (LCS diff, hunks), `markdown.ts`
    (response parser), `tree.ts` (selection/token sums), `utils.ts` (`cn()`).
  - `state/` — `AppProvider` holds all UI state; components read it with `useApp()`.
  - `lib/api.ts` calls the local `/api` routes; `lib/prompt.ts` builds each turn (rules + output
    format as system prompt; only files new to the conversation or changed since, tracked by hash in
    `ThreadState.fileVersions`); conversations continue via the Responses API's `previous_response_id`
    and are saved per project through `/api/threads` (Chats panel: search, rename, delete, delete all); `lib/changes.ts` turns a reply's
    `### FILE:` / `### EDIT:` / `### DELETE:` blocks into the Changes panel. To save output tokens the
    model returns existing files as SEARCH/REPLACE edit blocks (format in `FILE_FORMAT`, `lib/prompt.ts`,
    appended to the system prompt and deliberately not shown in Settings); `lib/edits.ts` rebuilds the
    whole file locally, and unplaceable edits are quietly re-requested in full (`/api/edit/repair`).
    The UI always shows whole files, never the edit format; `lib/images.ts` reads pasted/dropped
    pictures (scaled to 2048px, vision token estimate). Saving proposed changes is "Save" in the UI
    (code still calls it apply). `components/scm/` is a VS Code-style Source Control tab (stage, commit,
    push); `components/pr/` opens pull requests as a GitHub-style compare with base/compare branch dropdowns
    (base defaults to the branch's parent); `components/scm/BranchPicker` is the VS Code-style branch
    switcher opened from the status bar;
    both use `src/server/git.ts` (git + `gh`). The open folder is kept in sessionStorage so a reload
    (e.g. Vite reloading after saving into its own repo) reopens it. Settings are saved in localStorage.
  - `data/types.ts` — shared types; `data/defaults.ts` — output-format prompt, default exclusions,
    token estimates. Project files come from `src/server/fs` via `/api/project/*`.
  - `#open=<path>` opens a folder directly (bookmarkable).
- `src/server/` — Node code that runs inside the local Vite process (`plugin.ts` mounts `/api`).
  No dedicated backend server, no Next.js, no React Server Components. Talks to Azure AI Foundry
  through the `openai` SDK's `AzureOpenAI` client. See `src/server/README.md` for routes.
  Type-checked by `tsconfig.node.json`; `src/web` by `tsconfig.app.json`.
- Config: `.env.local` (gitignored), template in `.env.example`. Server-only variables have no
  `VITE_` prefix, so they never reach the browser. Never prefix secrets with `VITE_`.

## Rust port (in progress)
Native GPUI rewrite per `docs/plans/cross-platform-rust-port.md` (see §15 Progress), built alongside
the current app. Cargo workspace in `crates/`: `core` (headless logic, workspace host, Foundry client),
`protocol` (wire types, JSON-RPC), `theme` (generated from `tokens.json` by `build.rs`), `agent`
(WSL JSON-RPC server). Rust is in `~/.cargo/bin`.
- `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`.
- Windows/macOS type-check from WSL: `cargo clippy -p contextual-core --no-default-features --target x86_64-pc-windows-msvc --target-dir target/xcheck`
  (the `llm` feature needs a C cross-compiler for `ring`).
- Parity fixtures under `crates/core/tests/fixtures/` are recorded from the TS code with Node; when TS
  behaviour changes before cutover, re-record them.

## UI conventions
- Dark theme is the default (`class="dark"` on `<html>`); light theme via `class="light"`.
- Style only with design-token utilities (see UI design below); Tailwind's default palette,
  font sizes and shadows are removed, so `bg-red-500` or `text-sm` produce nothing.
- Prefer small, single-purpose components composed together; avoid boolean-prop explosions
  (see the `vercel-composition-patterns` skill). Merge classes with `cn()`.
- shadcn/ui is configured via `components.json` (`rsc: false`, aliases under `@/`);
  add primitives with `npx shadcn@latest add <component>`.

## UI design
The UI spec is the Contextual design system, saved locally in `docs/design/contextual/`
(source link and version in `SOURCE.md`). Read it before building UI:
1. `README.md` — principles, voice, visual foundations, code-rendering rules, layout.
2. `tokens.json` — design tokens for dark and light themes.
3. `components/*/README.md` — one spec per component and per screen.

Rules:
- `tokens.json` is the **only** source of colors, spacing, radii, shadows and type styles.
  Never hard-code hex values or invent new spacing.
- `src/web/styles/tokens.css` is a hand-written mirror of tokens.json (CSS variables per theme +
  Tailwind `@theme`). When tokens.json changes, update tokens.css to match. Token → utility mapping:
  - colors: `bg-surface-100`, `text-ink-muted`, `border-border-strong`, `bg-diff-add-word`,
    `text-syn-keyword`, `text-file-csharp`, … (raw CSS vars: `var(--ink)`, `var(--accent)`)
  - type: `text-ui-caption|ui-small|ui-body|ui-prose|ui-title|ui-display|code|code-small`
    (size, line height, weight and tracking together), `font-sans`, `font-mono`
  - spacing: base unit is `space-1` (4px), so `p-2` = space-2, `gap-6` = space-6; use only
    the token steps 1, 2, 3, 4, 6, 8. Heights: `h-row` (22px), `h-control` (26px)
  - radius: `rounded-sm|md|lg|full`; shadow: `shadow-popover|modal|window`
  - Fixed layout widths from the README (312px, 320px, 820px) use arbitrary values.
- shadcn's names (`bg-background`, `bg-primary`, `text-muted-foreground`, …) are aliased to
  tokens in `src/web/index.css`. shadcn's `accent` is not: here `accent` is the blue primary,
  so replace `bg-accent` in added shadcn components with `bg-surface-hover`, and restyle
  their sizes to the token type styles and `h-control`.
- `components/bundle.js` and `components/bundle.css` are **reference code** to port logic from
  (syntax tokenizer, LCS line diff + word ranges, collapsing unchanged regions). They are not
  production code: don't import, load or copy them wholesale into `src/`. Re-implement as
  typed React components and modules.

## Project skills (`.claude/skills/`, installed from skills.sh, tracked in `skills-lock.json`)
- `frontend-design` — visual direction and polish.
- `tailwind-design-system` — Tailwind v4 tokens and component patterns.
- `shadcn` — adding/composing shadcn/ui primitives.
- `vercel-composition-patterns` — reusable component APIs (compound components, etc.).
- `vercel-react-best-practices` — React performance. Ignore the Next.js / RSC / server-action
  rules; this is a Vite SPA.
- `web-design-guidelines` — UI/accessibility review.

Update skills with `npx skills update -p`.
