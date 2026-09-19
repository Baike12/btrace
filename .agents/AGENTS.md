# btrace — Langfuse PG-Only Fork (with Rust Backend)

本项目是 [Langfuse](https://langfuse.com) 的魔改分支：
- **存储层**：所有后端存储统一迁移到 PostgreSQL（ClickHouse/S3/Redis 已移除）
- **后端层**：队列消费者和 API Server 由 Rust (axum + sqlx) 实现，内存仅 ~7MB RSS
- **前端层**：Next.js 保留为纯前端 + SSR 渲染层 + Pages Router API 路由 (NextAuth auth 等)

## 本地环境

- PostgreSQL 14.18 (Homebrew)，连接串：`postgres://baike@127.0.0.1:5432/lanfuse`
- 数据库：`lanfuse`（如不存在需创建）
- 密码：由用户控制，不写入文件
- 测试账号：`qa-test@langfuse.dev` / `QaTest123!@#`（有 OWNER 权限，用于浏览器验证）

## How To Work

- Read the minimal local context required for the task.
- Keep changes scoped and avoid unrelated refactors.
- For bug fixes, write the failing test first, confirm it fails, then fix the
  bug. If the bug depends on a data shape, pause and ask whether it can be
  produced locally. (`pnpm run seed` is currently broken — see Known Issues —
  so today that means reusing the existing local database or fixing the CLI
  first; `packages/shared/scripts/seeder/AGENTS.md` describes the intended
  seeder design.)
- For user-visible frontend changes in `web/**`, review the affected flow in a
  real browser before signoff. Prefill the data the flow needs by whatever
  working means is available — never with ad-hoc scripts or direct SQL inserts
  into tables the app also writes.
- For documentation screenshots in Markdown, avoid fixed `height` on `<img>`
  tags; prefer Markdown images or width-only HTML so previews preserve aspect
  ratio.
- When working on the search bar or any filtering UI/grammar, read
  `web/src/features/search-bar/README.md` first. It owns the grammar ↔
  `FilterState` contract, the validate/lower parity invariants, and the
  cross-view extension playbook — the bar is intended to become the primary
  filter interface for every filterable view, so new filtering work extends it
  through that contract rather than forking it.
- Never commit secrets or credentials. Keep `.env*.example` files in
  sync with required env vars.

## Project Structure

```text
btrace/
|- langfuse-rs/              # Rust backend (axum API + queue consumers)
|  |- crates/                #   langfuse-core, -db, -queue, -ingestion, -api, -auth, -webhooks
|  |- src/queues.rs          #   register_all_queues — shared by both binaries
|  |- bin/server.rs          #   API + queue consumers in one process (what the image runs)
|  `- bin/worker.rs          #   queue consumers only (NOT shipped in the image)
|- web/                      # Next.js app (UI frontend, SSR pages, Pages Router)
|- packages/shared/          # Shared domain, DB schema (Prisma), queue contracts
|- generated/                # Generated API clients (do not hand-edit)
|- docs/                     # Architecture & design documents
|  `- lexqa-integration.md   # How LexQA consumes this instance (OTLP + public API)
|- docker/entrypoint.sh      #   supervises the two processes in the image
|- Dockerfile                #   single-container image (Rust server + Next.js)
`- scripts/                  # Repo scripts
```

- **Process model**: the runtime is two processes, not three —
  `bin/server.rs` is the "unified backend" and already hosts all five queue
  consumers via `tokio::spawn`. There is no separate worker in the container.
  Running `bin/worker.rs` alongside `bin/server.rs` gives every queue two pools of
  consumers competing for the same `pg_jobs` rows.
- **Ingest paths**: `POST /api/public/otel/v1/traces` (OTLP/HTTP, protobuf + JSON
  + gzip — what LexQA uses) and `POST /api/public/ingestion` (Langfuse SDK batch).
  Both write **synchronously** through `langfuse-ingestion/src/sink.rs`; the
  buffered `BatchWriter` was removed from these paths so a trace is queryable as
  soon as the request returns.
- **Cost**: `langfuse-ingestion/src/pricing.rs` fills `calculated_*_cost` and
  `cost_details` from `models` + `pricing_tiers` + `prices`. `cost_details` is
  keyed by **usage type** (so cache reads are their own line), values are JSON
  numbers, and a `total` key is always present — that is the shape the read path
  (`reduceUsageOrCostDetails`) and the UI both consume. Exactly one pricing tier
  is selected per observation (conditions, then priority, else default).
- **Auth**: two independent credential systems.
  - `/api/public/*` verifies an **API key** (`Basic base64(pk:sk)`), for machine
    clients. LexQA uses this.
  - The console's private `/api/*` routes verify a **session**: `POST
    /api/auth/login` checks `users.password` (bcrypt), issues a JWT signed with
    `JWT_SECRET`, and sets it as an `HttpOnly` cookie. `middleware/auth.rs`
    verifies it and injects a `Session`; every handler that takes a
    `project_id` then calls `require_project_access`, because that value comes
    from the request.
  - The two do not cross: an API key is not a session and a session is not an
    API key (`langfuse-api/tests/private_api_auth.rs` pins this).
  - `POST /api/auth/signup` closes with `LANGFUSE_DISABLE_SIGNUP=true`. The
    session cookie adds `Secure` only when `LANGFUSE_SECURE_COOKIES=true`.
  - Cross-origin access is off unless `LANGFUSE_CORS_ALLOWED_ORIGINS` lists the
    origins — the UI is same-origin through the Next.js proxy.
- **LexQA integration**: see `docs/lexqa-integration.md`.

- Dependency direction:
  - `web` -> `@langfuse/shared`
  - `@langfuse/shared` -> no imports from `web` or `langfuse-rs`
  - `langfuse-rs` -> connects directly to PostgreSQL via sqlx (independent of Prisma)
- Queue payload schemas and queue-name contracts are owned by
  `packages/shared/src/server/queues.ts`.
- Rust backend implements its own queue system (`langfuse-rs/crates/langfuse-queue/`)
  using `SELECT ... FOR UPDATE SKIP LOCKED` on the same `pg_jobs` table, making
  Node.js and Rust workers interoperable.
- High-signal shared entry points:
  - Domain models: `packages/shared/src/domain/{observations,traces,scores}.ts`
  - Postgres schema: `packages/shared/prisma/schema.prisma`
  - Rust domain types: `langfuse-rs/crates/langfuse-core/src/types.rs`
  - Rust queue system: `langfuse-rs/crates/langfuse-queue/src/consumer.rs`
  - Rust API routes: `langfuse-rs/crates/langfuse-api/src/app.rs`
- Architecture principles live in `.agents/ARCHITECTURE_PRINCIPLES.md`.
- **There are no Next.js API routes any more.** `web/src/pages/api/**` and the
  App Router `web/src/app/api.disabled/` were both removed when the backend moved
  to Rust; `web/src/app/` now holds only `layout.tsx`. Every `/api/*` request is
  either proxied to the Rust backend by the `rewrites()` block in
  `web/next.config.mjs` or served by `web/src/utils/api.ts`, a tRPC-shaped
  wrapper over the Rust REST API. Do not add Pages Router API routes back: the
  App Router interception problem that motivated `api.disabled/` still applies to
  Next.js 16.

## Core Commands

- Install deps: `pnpm install`
- Dev all packages: `pnpm run dev`
- Dev web only: `pnpm run dev:web`
- Rust build (debug): `cd langfuse-rs && cargo build`
- Rust build (release): `cd langfuse-rs && cargo build --release`
- Rust server (dev): `cd langfuse-rs && cargo run --bin server`
- Rust worker (dev): `cd langfuse-rs && cargo run --bin worker`
- Rust tests: `cd langfuse-rs && cargo test --workspace`
- Rust lint: `cd langfuse-rs && cargo clippy`
- Container build: `docker build -t btrace .` (context is the repo root).
  The image compiles `packages/shared` + `ee` and runs `prisma generate` itself —
  `.dockerignore` keeps host `dist/` out of the context, so without those steps
  `next build` cannot resolve their exports.
- Container run: see `docs/lexqa-integration.md` §3 — needs `DATABASE_URL` plus
  the `SALT` the API keys were hashed with
- Lint all: `pnpm run lint`
- Typecheck all: `pnpm run typecheck` / `pnpm tc`
- Build check: `pnpm run build:check`
- Full build: `pnpm run build`
- Worktree bootstrap: `bash scripts/codex/setup.sh`
- Worktree maintenance: `bash scripts/codex/maintenance.sh`
- Install Playwright Chromium: `pnpm run playwright:install`

## Verification

- `web/**`: `pnpm run lint` plus targeted web tests.
- `packages/shared/**` non-schema changes:
- `packages/shared/prisma/**`:
  `pnpm run lint`, `pnpm run db:generate`, and targeted web regressions.
- `langfuse-rs/**`: `cargo build`, `cargo test`, `cargo clippy`.
- Public API contracts: `langfuse-rs/crates/langfuse-api/src/routes/**` owns the
  implementation. `pnpm run lint` plus targeted Rust tests.
- Cross-package refactors: `pnpm run lint`, `pnpm run typecheck`, `cargo build`,
  and targeted tests for impacted packages.

## Known Issues

Debt that is deliberately carried, with the evidence to confirm it. Each item
names a command or file that reproduces the claim — check it before assuming the
entry is stale.

1. **Neither web nor shared typechecks or lints cleanly.** `npx tsc -p
   web/tsconfig.build.json --noEmit --skipLibCheck` reports ~385 errors
   (`pnpm run typecheck` delegates to the same config via tsgo), dominated by
   TS7006 (implicit `any` at call sites) because `web/src/utils/api.ts` returns
   `any` from its tRPC-shaped proxy. `pnpm run lint` exits 1 on
   `--max-warnings 0`: 8 warnings in web (`src/pages/auth/sign-in.tsx`,
   `sign-up.tsx`, `src/pages/project/[projectId]/settings/index.tsx`,
   `src/utils/api.ts`) and 32 in `packages/shared`. All of them predate this
   cleanup; none are in files the cleanup touched. `Dockerfile` sets
   `NEXT_IGNORE_BUILD_ERRORS=true` so image builds do not fail on the type
   errors. Error counts are only meaningful when compared as a set — line shifts
   make identical errors look new, so diff on (file, message) instead.

2. **`packages/shared/src/server/queries/clickhouse-sql/` is an alias layer, and
   part of it is a no-op.** The directory is 85 lines of re-exports from
   `pg-sql/` (641 lines, the real implementation) plus stubs. The stubs are the
   problem: `event-query-builder.ts` exports `EventsQueryBuilder` as an empty
   `class {}` and `buildEventsFullTableSplitQuery` as `return null`, and
   `orderByToEntries` returns `[]` — and `repositories/events.ts` (4074 lines,
   reachable from the UI) constructs that empty class at five sites. Consumers
   never got repointed at `pg-sql/`, so this is the highest-priority item: the
   code silently produces empty queries instead of failing. Nothing was deleted
   here because deciding whether `events.ts` is live requires its own audit.

3. **Several reachable views call procedures the Rust API does not serve.**
   `web/src/utils/api.ts` lists the real resources (`REAL_RESOURCES`); anything
   else falls through to a placeholder. Affected today: `api.annotationQueues.*`
   and `api.annotationQueueItems.*`, `api.dashboardWidgets.*` and
   `api.dashboard.*` (dashboards list over `/api/dashboards`, but the detail
   view cannot load its widgets), `api.members.*`, `api.scoreConfigs.*`,
   `api.llmApiKey.*` / `api.defaultLlmModel.*`, and `api.events.batchIO` (the v4
   events tables' input/output column). These pages render empty rather than
   erroring.

4. **The placeholder proxy is not silent.** `web/src/utils/api.ts` returns
   `{ data: undefined }` from placeholder `useQuery`s and `undefined` from
   placeholder `useMutation`s, logging a `console.warn`. A mutation whose
   `onSuccess` destructures its result throws `TypeError: Cannot read properties
   of undefined` — that is how the support form used to crash on submit. Prefer
   deleting a call site over tolerating one.

5. **The container runs no database migrations.** Neither `Dockerfile` nor
   `docker/entrypoint.sh` invokes `prisma migrate`; a fresh deployment must be
   migrated out of band. `docs/lexqa-integration.md` does not cover it.

6. **The seeder CLI is broken.** `pnpm run seed` fails with
   `ENOENT … tsx scripts/seeder/cli.ts`, and `packages/shared/package.json`'s
   `prisma.seed` points at `scripts/seeder/seed-postgres.ts`, which does not
   exist either. `packages/shared/scripts/seeder/` still holds `scenarios/`,
   `utils/` and `seed-dataset-versions.ts`, so the pieces are there but nothing
   wires them up. Until it is fixed, seed test data by hand or reuse the
   existing local database.

7. **Four dependencies have no remaining importers**:
   `@clickhouse/client`, `@aws-sdk/client-sesv2`, `nodemailer` and
   `@types/nodemailer` (the TypeScript email tree and the ClickHouse client
   stub were removed). `ioredis` looks similar but is *not* unused —
   `server/auth/apiKeys.ts` and `server/services/PromptService/index.ts` still
   import it. Removing the four is a `pnpm install` away but was left out to
   keep the cleanup commit off the lockfile.

## Generated Files

Do not hand-edit generated or build artifacts:

- `generated/*`
- `web/.next/*`
- `web/.next-check/*`
- `*/dist/*`
- `packages/shared/prisma/generated/*`

Public API contract changes are made in `langfuse-rs/crates/langfuse-api/**`.
Never hand-edit `generated/**`.

## Shared Agent Setup

- `.agents/AGENTS.md` is the canonical root guide.
- Root `AGENTS.md` is a symlink to `.agents/AGENTS.md`.
- Root `CLAUDE.md` is a compatibility symlink to `AGENTS.md`.
- When creating or editing `.agents/skills/**`, use
  `.agents/skills/skill-creator/SKILL.md`; keep skills concise with
  progressive disclosure.
- After changing shared agent setup, run `pnpm run agents:sync` and
  `pnpm run agents:check`.
- Generated provider config and shim outputs under `.claude/`, `.cursor/`,
  `.codex/`, `.vscode/`, or `.mcp.json` are local artifacts, not source of
  truth files.
