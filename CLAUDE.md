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
- Use the semantic color tokens (`bg-background`, `text-muted-foreground`, `border-border`, …)
  defined in `src/web/index.css` instead of raw palette colors.
- Prefer small, single-purpose components composed together; avoid boolean-prop explosions
  (see the `vercel-composition-patterns` skill). Merge classes with `cn()`.
- shadcn/ui is configured via `components.json` (`rsc: false`, aliases under `@/`);
  add primitives with `npx shadcn@latest add <component>`.

## Project skills (`.claude/skills/`, installed from skills.sh, tracked in `skills-lock.json`)
- `frontend-design` — visual direction and polish.
- `tailwind-design-system` — Tailwind v4 tokens and component patterns.
- `shadcn` — adding/composing shadcn/ui primitives.
- `vercel-composition-patterns` — reusable component APIs (compound components, etc.).
- `vercel-react-best-practices` — React performance. Ignore the Next.js / RSC / server-action
  rules; this is a Vite SPA.
- `web-design-guidelines` — UI/accessibility review.

Update skills with `npx skills update -p`.
