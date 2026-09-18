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
  bug. If the bug depends on a data shape, pause and ask: can
  `pnpm run seed` prefill that shape locally? If not, consider extending a
  seeder scenario so the bug stays cheaply reproducible
  (`packages/shared/scripts/seeder/AGENTS.md`), or note why a seed cannot
  express it.
- For user-visible frontend changes in `web/**`, review the affected flow in a
  real browser before signoff. Prefill the data the flow needs with the seed
  CLI (`pnpm run seed -- list` shows scenarios; runs print UI deep links) —
  never with ad-hoc scripts or raw ClickHouse inserts.
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
|- ee/                       # Enterprise package consumed by web
|- generated/                # Generated API clients (do not hand-edit)
|- fern/                     # API definition sources
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
  - `web` -> `@langfuse/shared`, `@langfuse/ee`
  - `@langfuse/ee` -> `@langfuse/shared`
  - `@langfuse/shared` -> no imports from `web`, `langfuse-rs`, or `ee`
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
- **Important**: `web/src/app/api/` has been renamed to `web/src/app/api.disabled/`
  because App Router API routes interfere with Pages Router dynamic routes under
  `/api/` in Next.js 16. All API routes must live under `web/src/pages/api/`.
  App Router UI features (layout, loading, error boundaries) are unaffected.

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
- Public API contracts in `web/src/pages/api/public/**`,
  `web/src/features/public-api/types/**`, or `fern/apis/**`: `pnpm run lint`,
  targeted server API tests, and Fern update/regeneration.
- Cross-package refactors: `pnpm run lint`, `pnpm run typecheck`, `cargo build`,
  and targeted tests for impacted packages.

## Generated Files

Do not hand-edit generated or build artifacts:

- `generated/*`
- `web/.next/*`
- `web/.next-check/*`
- `*/dist/*`
- `packages/shared/prisma/generated/*`

Public API contract changes must update Fern sources in `fern/apis/**` and
regenerated outputs. Never hand-edit `generated/**`.

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
