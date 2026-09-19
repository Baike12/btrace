# Agent Guidelines for `web`

This file covers package-local guidance for this package.
Use root [AGENTS.md](../AGENTS.md) for monorepo-level rules, and read its
**Known Issues** section before assuming a failing command is your fault.

## Purpose

- Next.js app that renders the UI and does SSR. It is **frontend only**.
- There is no tRPC backend and there are no Next.js API routes. `src/server/**`,
  `src/features/*/server/**`, `src/pages/api/**` and `src/app/api.disabled/`
  were all removed in the move to the Rust backend; `src/app/` now holds only
  `layout.tsx`. Do not add them back — the App Router interception problem that
  forced `api.disabled/` still applies on Next.js 16.
- The backend is `langfuse-rs/` (axum + sqlx). It listens on **:8010**
  (`RUST_API_URL`) and talks to Postgres directly.
- Check `web/package.json` for the current Next.js, React and Vitest versions
  before version-sensitive work.

## How web talks to the backend

Two layers, both in `web/`:

1. `next.config.mjs` → `rewrites()` proxies `/api/<resource>/*`,
   `/api/project/*`, `/api/auth/{login,signup,session,logout}` and
   `/api/public/*` to `RUST_API_URL`. Only `/api/trpc/*` and NextAuth used to be
   local; neither exists any more.
2. `src/utils/api.ts` is a tRPC-shaped wrapper over that REST surface, so
   existing call sites (`api.traces.all.useQuery(...)`) kept working across the
   migration.

`src/utils/api.ts` is the single most important file in this package. It maps
procedure names onto REST verbs through `REAL_RESOURCES`; **anything not in that
map resolves to a placeholder that returns `undefined`** and only logs a
`console.warn`. When adding a call site, confirm the resource and procedure are
in `REAL_RESOURCES` first, and see Known Issues for the views that currently are
not.

## High-Signal Entry Points

- App shell/providers: `src/pages/_app.tsx`
- Data access: `src/utils/api.ts`
- Layout, navigation registry and filtering: `src/components/layouts/routes.tsx`,
  `src/components/layouts/app-layout/**`
- Feature modules: `src/features/*`
- Reusable UI components: `src/components/*`
- Trace rendering: `src/components/trace/**`
- Tables, filters and the grammar search bar: `src/components/table/**`,
  `src/features/filters/**`, `src/features/search-bar/**`

## Shared Package Imports

- Prefer `@langfuse/shared` in frontend-safe web code for shared types, zod
  schemas, domain contracts, table definitions, prompt/eval/model-pricing
  helpers, and other cross-runtime utilities.
- Use `@langfuse/shared/src/server` only where a server-only helper is genuinely
  needed (SSR pages, `getServerSideProps`, test setup). It pulls in the server
  tree, so keep it out of client components.
- Use narrower subpaths such as `@langfuse/shared/src/env` or
  `@langfuse/shared/encryption` when that focused surface is the clearest
  dependency.
- See `../packages/shared/AGENTS.md` for the full shared export map.

## Web Conventions

- Put net-new feature code under `src/features/<feature>/*`; put broadly
  reusable components under `src/components/*`.
- Prefer Shadcn/ui primitives from `src/components/ui`; if a missing component
  must be installed, ask the user before doing so.
- Tailwind is the default styling layer; use the shared palette and globals in
  `src/styles/globals.css`.
- In flex layouts, prefer `gap-*` over margin-based `space-x-*`/`space-y-*`.
- Treat `!` Tailwind classes as a smell. Step back and fix the owning layout,
  variant, or primitive before overriding with higher specificity.
- When changing shared UI/table patterns, update sibling variants consistently,
  including default-visible and hidden columns or states.
- For component style variants, prefer `cva` with `VariantProps` and merge
  caller classes through `cn`, following existing `src/components/ui/*`
  components:

  ```tsx
  const cardVariants = cva("rounded-md border", {
    variants: {
      intent: { default: "bg-background", error: "border-destructive" },
    },
    defaultVariants: { intent: "default" },
  });

  type CardProps = React.HTMLAttributes<HTMLDivElement> &
    VariantProps<typeof cardVariants>;

  const className = cn(cardVariants({ intent }), props.className);
  ```

- When anchoring sticky, fixed, or absolute elements to the viewport, use
  `top-banner-offset`, `pt-banner-offset`, `h-screen-with-banner`, or
  `min-h-screen-with-banner` instead of raw `top-0` so banners do not overlap
  the UI.
- **Z-index / layers — key idea: we are migrating from z-indexes to a layer
  system** (start of a developing design system; extend it, don't work around
  it). **To put something on top of something else, use a layer, not a
  z-index.** The app renders inside `#__next`, isolated into one stacking
  context (`globals.css`), so its z-indexes can't escape; overlays go in layers
  that sit outside it and always win. Today there's one layer, `tooltip`:
  containers declared in `_document.tsx`, ordered by `LAYER_ORDER` (later = on
  top), no z-index; render via `<Layer name="…">` (`src/components/ui/layer.tsx`).
  Add a layer only when an overlay must escape the app (else use a Radix
  `*.Portal`) by adding a name to `LAYER_ORDER`. z-index stays local to a layer
  or component (1–2 max), never to escape the app.
- Public API endpoints are implemented in `../langfuse-rs/crates/langfuse-api/`,
  not here. The web side only consumes them through `src/utils/api.ts`.

## Tests

- Client tests: `src/**/*.clienttest.ts(x)` (`pnpm --filter web run test-client`).
- In-source tests: `vitest` blocks inside a module
  (`pnpm --filter web run test:in-source`).
- E2E: `src/__e2e__/*` (`pnpm --filter web run test:e2e` for Playwright specs,
  `test:e2e:server` for the vitest-driven suite).
- Server tests (`src/__tests__/server/**`, `*.servertest.ts`) were removed with
  the backend. The vitest `server*` projects still exist in `vitest.config.mts`
  and match nothing; `pnpm --filter web run test` therefore reports no tests for
  them.
- Keep tests independent; no reliance on test execution order.
- Confirm the target `*.clienttest.*` file exists before passing a pattern to
  `vitest run`; source files do not always have a matching colocated test file.
- When passing a Vitest file or pattern through `pnpm --filter web ...`, make it
  relative to `web/` because the script runs with `web` as the working
  directory. Example: use `src/features/search-bar/lib/validate.clienttest.ts`,
  not `web/src/...`.
- Prefer separate test files for components and integration coverage; use Vitest
  in-source tests mainly for small-scoped utilities.
- Do not extract private utility functions into separate files only to make them
  testable. Keep them local unless the user explicitly asks for extraction or the
  utility is meaningfully reused.

## Quick Commands

- Dev: `pnpm --filter web run dev`
- Lint: `pnpm --filter web run lint` (fails on any warning — `--max-warnings 0`)
- Lint fix: `pnpm --filter web run lint:fix`
- Typecheck: `pnpm --filter web run typecheck` (see Known Issues: ~385 errors)
- Client tests: `pnpm --filter web run test-client <args>`
- In-source tests: `pnpm --filter web run test:in-source <args>`
- E2E tests: `pnpm --filter web run test:e2e`
- Build: `pnpm --filter web run build`
- Storybook: `pnpm --filter web run storybook`

## Playbooks

### Add a backend call

1. Confirm the Rust route exists in `../langfuse-rs/crates/langfuse-api/src/routes/`.
2. If `src/utils/api.ts` does not already route that resource, add it to
   `REAL_RESOURCES` (or a real-proc branch) — do not rely on the placeholder.
3. Call it through `api.<resource>.<procedure>` so query keys and invalidation
   keep working.

### Add a frontend feature

1. Prefer `src/features/<feature>/*` for feature-local code.
2. Put broadly reusable components in `src/components/*`.
3. Review the affected user flow in a real browser before signoff — see the
   shared browser-review workflow in
   [`../.agents/skills/frontend-browser-review/SKILL.md`](../.agents/skills/frontend-browser-review/SKILL.md).
   For large features, virtualized lists, or local state architecture also read
   [`../.agents/skills/frontend-large-feature-architecture/SKILL.md`](../.agents/skills/frontend-large-feature-architecture/SKILL.md).

### Error handling

1. Throw `BaseError` subclasses (e.g. `LangfuseNotFoundError`) and let the
   caller surface them; extend `../packages/shared/src/errors/` as needed.
2. `src/utils/trpcErrorToast.tsx` renders the toast for failed queries.

## Package-Specific Rules

- Router style is Pages Router-centric; follow existing routing patterns.
- In `src/pages`, do not keep both `foo.ts(x)` and a `foo/` folder. If the
  folder exists, put the route implementation in `foo/index.ts(x)` instead.
- Do not hand-edit build artifacts: `.next/*`, `.next-check/*`, `dist/*`.
