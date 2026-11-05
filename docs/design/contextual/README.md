# Contextual — design system

Contextual is a local desktop chat client for code. You tick the project files the model should see, ask for a change, review the proposed edits as a GitHub-style diff, and apply what you want. The model talks to a self-hosted Azure AI Foundry deployment and **never writes to disk**: every write is an explicit, deterministic action by the user (Apply, Copy file content, Copy diff), and every write can be undone.

The visual language borrows from VS Code and GitHub's pull-request view on purpose: the people using Contextual live in those tools all day, so code here looks and reads exactly like code there. Dark is the default theme; Light is a full variant.

The **Screens** cards show the app end to end, using a sample .NET clean-architecture solution, `Inventory.Api`:

| Screen | State shown |
| --- | --- |
| `ScreenWelcome` | Empty state, no project opened, recent projects |
| `ScreenWorkspace` | Three columns: 12 files selected, a finished response with 5 file changes |
| `ScreenDiff` | Diff view open full-width over the chat, `Order.cs` expanded, a conflict on `OrderEndpoints.cs` |
| `ScreenDiffSplit` | The same diff in split layout: old left, new right, like GitHub |
| `ScreenCodePreview` | Read-only preview of `Order.cs` from the explorer; context bar near the limit (amber) |
| `ScreenApply` | Apply dialog: create / overwrite / delete per file, backup option |
| `ScreenApplied` | Toast "4 files written" with Undo |
| `ScreenSettings` | Foundry endpoint and deployments, exclusion globs, output-format system prompt |

## Principles

1. **The user writes, not the model.** Nothing touches disk without a button press that names the count ("Apply 4 files"). The status bar says so all the time: *Read-only: nothing is written until you apply*.
2. **Show the cost before it's spent.** Every file shows its token count; the context bar shows the total, the share of the deployment's limit and an estimated price per message, and it turns amber at 75% and red at 95%.
3. **Code looks like code.** Real syntax highlighting, line numbers, indent guides, and diffs with word-level highlights. Never plain text, never placeholder lines.
4. **Dense but calm.** VS Code row density (22px rows, 13px text), hairline borders instead of shadows, one accent colour. Colour is spent on code and on status, not on chrome.
5. **Stale is visible.** A file changed on disk since it was sent gets a badge in the explorer; a proposal made against an older version gets a conflict banner and is left out of Apply until reviewed.

## Voice

- Sentence case everywhere. Pane titles are the one exception (`EXPLORER`, `CHANGES`, uppercase caption style).
- Buttons are verbs, with the count when there is one: "Apply 4 files", "Expand 18 lines", "Copy diff". Never "OK" or "Submit".
- State facts plainly: "Conflict: file modified on disk since context was loaded." No exclamation marks, no "Oops".
- Paths, globs, model names and token counts are monospace; everything else is the UI sans.
- Numbers: `~11,200 tokens` in full in the context bar, abbreviated `1.5k` in trees and lists. Prices are estimates and say so with `~`.

## Visual foundations

**Surfaces.** Four steps. `surface-000` is chrome (title bar, side bars, status bar), `surface-100` is the editor ground (chat, code, diffs, dialogs), `surface-200` lifts things inside it (code block headers, diff file headers, the input box), `surface-300` is controls at rest. Panes are separated by a `border` hairline, never by shadow; `shadow-popover` and `shadow-modal` are only for things that float.

**Colour.** One accent (`accent`, VS Code blue) for the primary action, checked state, active tab and focus. Status colours carry fixed meanings and are never decoration: `success` = added / OK, `modified` = M, `danger` = deleted / over limit, `warning` = near limit, changed on disk, conflict. Each status is also marked by a letter (A / M / D) or an icon, so colour is never the only signal.

**Syntax.** `syn-*` tokens reproduce VS Code Dark+ in the dark theme and GitHub Light in the light theme: keyword, control, type, function, string, number, comment, variable, tag, heading. Comments are italic. Language is detected from the file extension or the code fence: C#, TypeScript/JavaScript, Python, JSON, YAML, XML/.csproj, SQL, Markdown, shell.

**Diffs.** Added and removed lines get `diff-add-bg` / `diff-del-bg`; their line-number cells get the stronger `*-gutter` tint; the changed characters inside a paired line get `diff-add-word` / `diff-del-word`, exactly like GitHub. Syntax colours stay on top of the tints. Two layouts, switched in the diff toolbar and remembered per user: **Unified** (one column, old and new line numbers) and **Split** (old on the left, new on the right, each with its own gutter; changed lines paired row by row, empty `diff-empty-bg` filler opposite pure additions or deletions, long lines wrap within their half). Unchanged regions collapse into `diff-hunk-bg` rows: "Expand 24 lines" followed by the `@@` header and the enclosing member.

**Type.** Inter for UI (13px body, 14px chat prose, 12px meta, 11px uppercase captions); JetBrains Mono for code at 13px / 20px with tab width 4 and ligatures off, falling back to Cascadia Code and Consolas on Windows. Both load from Google Fonts.

**Spacing and shape.** A 4px base (`space-1` … `space-8`), `row-height` 22px for trees and lists, `control-height` 26px. Radii are small: `radius-sm` 3px for checkboxes and tags, `radius-md` 6px for buttons, code blocks and diff cards, `radius-lg` 10px for dialogs, toasts and the window.

## Code rendering rules

- Every code surface uses the same renderer (`Contextual.codeView`): muted gutter with line numbers, a subtle current-line band (`editor-line-current`) with the active number in `editor-gutter-active`, and 1px indent guides every 4 columns (`editor-indent-guide`).
- Chat code blocks (`Contextual.codeBlock`) get a header bar: file glyph + full path (or the language label), the language, **Wrap lines** and **Copy**. Long blocks are cut with "Show N more lines".
- When the model returns files, each block is preceded by its `### FILE: <path>` header line, which is what the parser reads to build diffs (see Settings → Output format).
- Inline code in prose (`IOrderRepository`) uses the `pill-bg` pill in the mono face at 12px.
- Clicking a file in the explorer opens a read-only tab with the same renderer, a breadcrumb, encoding, token count and Copy / Wrap lines.

## Layout

The window is a title bar, a three-column body and a 24px status bar.

- **Left, 312px:** Files tab (explorer with tri-state checkboxes, token counts, exclusions, changed-on-disk badges, search, Select all / Clear, Respect .gitignore) and a collapsible **Skills & Rules** section below it (auto-detected AGENTS.md, CLAUDE.md, `.github/copilot-instructions.md`, `.cursor/rules/*`, `.claude/skills/*/SKILL.md`, each with a toggle and a Rule/Skill tag).
- **Centre, flexible:** tabs (Chat plus read-only previews), the chat thread at a 820px reading width, and the composer: context bar, input, deployment picker, *Resend full context* / *Only send new message*.
- **Right, 320px:** the Changes panel, shown when a response contains file edits: counts by status, per-file checkbox, +/− counts, conflict note, **Open diff** and **Apply N files…**.
- **Diff view** replaces centre and right columns (the explorer stays): a toolbar with Back to chat, Unified / Split, Expand all and Apply; a file list; then one card per file with Copy file content, Copy diff and *Include in apply*.

## Iconography

16px line icons drawn inline with `currentColor` at a 1.3px stroke, codicon-like: chevrons, folder, file, copy, wrap, refresh, warning, check, split/unified, undo. File types use small coloured glyphs instead of pictures (`C#` in `file-csharp`, `{ }` in `file-json`, `</>` in `file-xml`, `M↓` in `file-markdown`). Status uses boxed letters A / M / D. There is no logo artwork beyond the small two-sheet mark drawn in tokens (`accent` sheet, outlined sheet, `success` lines).

## Using the kit

`components/bundle.js` is a dependency-free script that sets `window.Contextual`. It contains the highlighter, the diff engine (line LCS + word ranges), and render functions that return HTML strings: `codeView`, `codeBlock`, `diffFile`, `fileTree`, `rulesPanel`, `contextBar`, `composer`, `chatThread`, `changesPanel`, `applyDialog`, `toast`, plus `screens.welcome / workspace / diff / settings`. `Contextual.mount(el, html)` inserts the HTML and wires the small interactions (collapse, wrap, copy, checkboxes, segmented controls). All styling is in `components/bundle.css` and reads only tokens. Sample data lives in `Contextual.samples`.

## Accessibility

- Text tokens meet 4.5:1 on every surface in both themes, including every `syn-*` colour on `surface-100` (lowest: `syn-comment` 4.9:1 dark). `ink-faint` is reserved for disabled and excluded items.
- Control outlines (`border-strong`), the focus ring and status letters meet 3:1.
- Diff state is carried by the `+` / `−` sign column and the line-number columns as well as colour; context-bar state adds "Near limit" / "Over limit" text and an icon.
- Focus is a 1px `focus-ring` outline with a 1px offset on every control.
