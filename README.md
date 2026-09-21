# btrace

A fork of [Langfuse](https://langfuse.com) with **all backend storage moved to PostgreSQL**
and the **API server plus queue consumers rewritten in Rust** (axum + sqlx). ClickHouse,
Redis and S3 are gone; the runtime is a single container with two processes.

| | |
|---|---|
| Storage | PostgreSQL only — traces, observations, scores, and the job queue (`pg_jobs`) |
| Backend | Rust: `langfuse-rs/` — API on `:8010` and five in-process queue consumers |
| Frontend | Next.js on `:3000`, proxying `/api/public/*` to the API |
| Ingest | `POST /api/public/otel/v1/traces` (OTLP/HTTP: protobuf, JSON, gzip) and `POST /api/public/ingestion` |

The wire contract is intentionally unchanged: `LANGFUSE_*` environment variables, the
`langfuse.*` OTLP span attributes, the `pk-lf-` / `sk-lf-` key format, and every
`/api/public/*` path behave as upstream, so existing instrumentation keeps working
untouched. What changed is the storage engine and the backend implementation behind them.

## Architecture

```text
                  :3000                          :8010
  browser ──► Next.js (UI + SSR) ──rewrites──► Rust API (axum)
                    │                              │
                    │                       ┌──────┴───────┐
                    │                       │  ingestion   │  synchronous write:
                    │                       │  sink.rs     │  queryable on return
                    │                       ├──────────────┤
                    │                       │ 5 queue      │  SELECT … FOR UPDATE
                    │                       │ consumers    │  SKIP LOCKED on pg_jobs
                    │                       └──────┬───────┘
                    └──────────────────────────────┴──► PostgreSQL
```

Two processes in one image, not three. `bin/server.rs` is the unified backend and
already hosts all five queue consumers via `tokio::spawn`; `bin/worker.rs` exists but is
**not** shipped, and running it alongside the server would give every queue two pools of
consumers competing for the same `pg_jobs` rows.

Both ingest paths write **synchronously**, so a trace is queryable as soon as the request
returns — there is no buffered batch writer in front of them.

## What differs from upstream

- **Storage**: every ClickHouse, S3 and Redis dependency is gone. Traces, observations,
  scores and the job queue live in PostgreSQL.
- **Queue**: `langfuse-rs/crates/langfuse-queue/` implements `SELECT … FOR UPDATE SKIP
  LOCKED` directly on the upstream `pg_jobs` table, so Node.js and Rust workers stay
  interoperable.
- **Cost**: `langfuse-ingestion/src/pricing.rs` fills `calculated_*_cost` and
  `cost_details` from `models` + `pricing_tiers` + `prices`. `cost_details` is keyed by
  usage type and always carries a `total` key.
- **Frontend**: no Next.js API routes remain. `web/src/app/` holds only `layout.tsx`
  and `web/src/utils/api.ts` is a tRPC-shaped wrapper over the Rust REST API.

## Repository layout

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
|- docker/entrypoint.sh      #   supervises the two processes in the image
|- Dockerfile                #   single-container image (Rust server + Next.js)
`- scripts/                  # Repo scripts
```

## Local development

```bash
pnpm install                                  # install workspace deps
pnpm --filter @langfuse/shared run build      # required after deleting packages/shared/dist
cd langfuse-rs && cargo run --bin server      # Rust API + queue consumers on :8010
pnpm run dev:web                              # Next.js on :3000
```

Point `DATABASE_URL` at a PostgreSQL 14+ database with the Prisma migrations applied.

Local service details, the test account and the current environment live in
[`AGENTS.md`](AGENTS.md).

## Build and run the container

```bash
docker build -t btrace:latest .               # context is the repo root
docker run -d --name btrace \
  -p 3000:3000 -p 8010:8010 \
  -e DATABASE_URL="postgres://<user>:<pass>@<host>:5432/<db>" \
  -e SALT="<openssl rand -base64 32>" \
  -e NEXTAUTH_SECRET="<openssl rand -base64 32>" \
  -e ENCRYPTION_KEY="<openssl rand -hex 32>" \
  btrace:latest
```

| Variable | Default | Notes |
|---|---|---|
| `DATABASE_URL` | — | **Required.** PostgreSQL connection string |
| `SALT` | `dev-salt` | Salt for the API-key fast hash. **Must match the value used when the keys were written**, or every key check fails |
| `NEXTAUTH_SECRET` | — | UI session signing |
| `ENCRYPTION_KEY` | — | Encryption for sensitive configuration |
| `LANGFUSE_BIND_ADDRESS` | `0.0.0.0` | Rust API bind address. Do not use `HOSTNAME` — Docker sets it to the container ID |
| `PORT` | `8010` | Rust API port. Pass it to the Rust process only: Next.js reads `PORT` too, and a shared value makes web fight for 8010 (`EADDRINUSE`) |
| `WEB_PORT` | `3000` | Next.js port |
| `RUST_API_URL` | `http://localhost:8010` | Next.js proxy target |
| `LANGFUSE_INIT_*` | — | Create org/project/API key/user on first boot |
| `LANGFUSE_DISABLE_SIGNUP` | `false` | `true` closes `POST /api/auth/signup`. Turn on when accounts come from `LANGFUSE_INIT_*` |
| `LANGFUSE_SECURE_COOKIES` | `false` | Set `true` behind TLS. **Leave off for plain HTTP**: browsers drop `Secure` cookies, which shows up as "login succeeds but every refresh logs out" |
| `LANGFUSE_CORS_ALLOWED_ORIGINS` | empty (off) | Comma-separated allowed origins. The UI is same-origin through the Next.js proxy, so this is normally unnecessary |

**The container runs no database migrations.** A fresh deployment must be migrated out of
band before first boot.

## Documentation

- [`docs/rust-backend-design.md`](docs/rust-backend-design.md) — backend design and internals
- [`docs/lexqa-integration.md`](docs/lexqa-integration.md) — consuming this instance from
  another service (OTLP + public API)
- [`AGENTS.md`](AGENTS.md) — architecture rules, commands, and the known-issues list

## Verification

```bash
pnpm run lint                 # web + packages/shared
pnpm run typecheck            # see AGENTS.md — the tree does not typecheck cleanly yet
cd langfuse-rs && cargo test --workspace && cargo clippy
```

## License

MIT, inherited from [Langfuse](https://github.com/langfuse/langfuse). See [`LICENSE`](LICENSE).
