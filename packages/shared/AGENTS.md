# Codex Guidelines for `@langfuse/shared`

This file covers package-local guidance for this package.
Use root [AGENTS.md](../../AGENTS.md) for monorepo-level rules.

## Purpose

- Shared domain, database, queue, and server utilities used by `web` and
  `langfuse-rs` (Rust backend).
- Primary owner of Postgres schema and queue payload contracts.
- ClickHouse/Redis/S3 code has been removed for the PG-only fork.

## Maintenance Contract

- `AGENTS.md` is a living document.
- Update this file in the same PR for material shared-package changes:
  - new/renamed schema or migration workflows
  - new/renamed queue contracts
  - changed exported surfaces or validation commands
- Because this package is consumed by both `web` and `langfuse-rs`, cross-package
  changes usually require updates in root `AGENTS.md` too.

## High-Signal Entry Points

- Main exports: `src/index.ts`
- DB clients and types: `src/db.ts`
- Server exports: `src/server/index.ts`
- Server cache utilities: `src/server/cache/*`
- Domain model types: `src/domain/*`
- Repository layer: `src/server/repositories/*`
- Queue payload schemas: `src/server/queues.ts`
- PG queue system: `src/server/pg-queue/*` (PgQueue, PgWorker, pgCache, pgSeenEvents)
- Dashboard/monitor query feature (data model + server-only builder/executor): `src/features/query/*`
- Postgres schema: `prisma/schema.prisma`
- For unstable public eval APIs, the public `evaluatorId` is currently the
  exact `EvalTemplate.id`. Latest-version family grouping is derived from
  `(projectId, name)` rather than stored on extra evaluator identity fields.
- Prisma migrations: `prisma/migrations/*`
- Seeder and support scripts: `scripts/seeder/*`

## Export Entry Points

- `@langfuse/shared` via `src/index.ts`: default shared surface for
  cross-runtime types, zod schemas, table definitions, domain models, prompt
  helpers, eval/model-pricing helpers, and other frontend-safe utilities.
- `@langfuse/shared/src/server` via `src/server/index.ts`: server-only barrel
  for shared backend services, repositories, queue helpers/contracts, PG queue
  helpers, auth helpers, logger/instrumentation, ingestion helpers,
  LLM execution helpers, and server test utilities.
- `@langfuse/shared/src/db` via `src/db.ts`: Prisma client singleton plus
  Prisma namespace/types for direct database access. Never route this into
  frontend-safe code.
- `@langfuse/shared/src/env` via `src/env.ts`: validated shared environment
  schema/accessors used by backend runtimes and scripts.
- `@langfuse/shared/encryption` via `src/encryption/index.ts`: encryption and
  signature helpers for secrets and signed payloads.
- `@langfuse/shared/query` via `src/features/query/index.ts`: dashboard query feature.
- Narrower exported subpaths also exist for targeted imports:
  `@langfuse/shared/src/server/auth/apiKeys`,
  `@langfuse/shared/src/server/ee/ingestionMasking`, and
  `@langfuse/shared/src/utils/chatml`.

When changing export surfaces, keep `package.json#exports`, the relevant barrel
file (`src/index.ts`, `src/server/index.ts`, etc.), and this guide aligned in
the same PR.

## Architecture Handbook

- For the cross-package system view, read the Rust backend design document:
  `../../docs/rust-backend-design.md`.
- Consult it when changing shared contracts that affect the web frontend,
  Rust backend, ingestion flow, or storage-layer boundaries.

## Quick Commands

- Dev watch build: `pnpm --filter @langfuse/shared run dev`
- Lint: `pnpm --filter @langfuse/shared run lint`
- Lint fix: `pnpm --filter @langfuse/shared run lint:fix`
- Typecheck: `pnpm --filter @langfuse/shared run typecheck`
- Build: `pnpm --filter @langfuse/shared run build`
- Prisma generate: `pnpm --filter @langfuse/shared run db:generate`
- Prisma migrate (dev): `pnpm --filter @langfuse/shared run db:migrate`

## Playbooks

### Postgres schema change

1. Update `prisma/schema.prisma`.
2. Add migration in `prisma/migrations/*`.
3. Regenerate client/types via `db:generate`.
4. Update affected repository/query code under `src/server/repositories/*`.
5. Add/adjust `web` and/or `worker` tests for changed behavior.

### Queue payload contract change

1. Update zod schemas/types in `src/server/queues.ts`.
2. Update queue helpers in `src/server/pg-queue/*` if queue names/payload
   handling changed.
3. Update producer and consumer code in `web` and `langfuse-rs`.
4. Add or update regression tests in affected packages.

### Export surface change

1. Decide whether the symbol belongs in the client-safe root barrel, the
   server-only barrel, or a narrower subpath export.
2. Update the owning file (`src/index.ts`, `src/server/index.ts`, `src/db.ts`,
   `src/env.ts`, or another explicit subpath).
3. Update `package.json#exports` if the public import path changed or a new
   subpath is required.
4. Update import sites in `web` and `ee` to use the intended
   entrypoint.
5. Update this file and any consuming package `AGENTS.md` guidance when the
   recommended import path changes.

## Package-Specific Rules

- Keep backward compatibility in queue payloads when possible.
- Do not hand-edit generated artifacts under `prisma/generated/*` or `dist/*`.
- Avoid exposing server-only modules through `src/index.ts` if they must remain
  frontend-safe.
