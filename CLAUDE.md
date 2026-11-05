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
  - `components/ui/` — small, generic, reusable primitives (shadcn/ui style).
  - `components/` — app-specific composite components built from the primitives.
  - `lib/utils.ts` — `cn()` helper (clsx + tailwind-merge).
- `src/server/` — Node code that runs inside the local Vite process (Vite plugin / middleware).
  No dedicated backend server, no Next.js, no React Server Components.

## UI conventions
- Dark theme is the default (`class="dark"` on `<html>`); light theme via `class="light"`.
- Use the semantic color tokens defined in `src/web/index.css` instead of raw palette colors.
  Those tokens must mirror `docs/design/contextual/tokens.json` (see UI design below).
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
  Map its tokens into `src/web/index.css` (CSS variables + `@theme`) and use them through
  Tailwind; never hard-code hex values or invent new spacing.
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
