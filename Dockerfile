# btrace (Langfuse PG-only fork) — single-container image.
#
# Runs the two processes this fork needs and nothing else:
#
#   * `langfuse-server` — Rust (axum) API + queue consumers, :8010
#   * `node web/server.js` — Next.js UI, :3000, proxying /api/public/* to :8010
#
# There is no separate worker process: `bin/server.rs` already hosts the queue
# consumers in-process, and `bin/worker.rs` is therefore not copied in. Running
# both would register every queue twice and have two pools of workers competing
# for the same `pg_jobs` rows.
#
# Everything is persisted in PostgreSQL, so the container is stateless — supply
# `DATABASE_URL` pointing at any reachable instance. LexQA reuses its own
# Postgres with a dedicated `langfuse` database, which is what the upstream
# docker-compose profile did too.
#
# Build:   docker build -t btrace .
# Run:     docker run -p 3000:3000 -p 8010:8010 \
#            -e DATABASE_URL=postgres://user:pass@host:5432/langfuse \
#            -e SALT=... -e ENCRYPTION_KEY=... \
#            -e LANGFUSE_INIT_ORG_ID=LexQA \
#            -e LANGFUSE_INIT_PROJECT_ID=LexQA \
#            -e LANGFUSE_INIT_PROJECT_PUBLIC_KEY=pk-lf-lexqa-init \
#            -e LANGFUSE_INIT_PROJECT_SECRET_KEY=sk-lf-lexqa-init \
#            btrace

# syntax=docker/dockerfile:1.7

# ============================================================================
# Stage 1 — Rust backend (API + queue consumers)
# ============================================================================
FROM rust:1.90-bookworm AS rust-build

WORKDIR /src
COPY langfuse-rs/ ./langfuse-rs/

RUN cd langfuse-rs \
    && cargo build --release --bin server \
    && strip target/release/server

# ============================================================================
# Stage 2 — Next.js UI (standalone output)
# ============================================================================
FROM node:24-bookworm-slim AS web-build

ENV PNPM_HOME=/pnpm \
    PATH=/pnpm:$PATH \
    NEXT_TELEMETRY_DISABLED=1

RUN corepack enable && corepack prepare pnpm@11.4.0 --activate

WORKDIR /src

# Manifests first, so `pnpm install` is cached independently of source edits.
# `patches/` is part of the dependency graph — pnpm applies it from
# patchedDependencies during install, and omitting it fails the install.
COPY pnpm-workspace.yaml pnpm-lock.yaml package.json turbo.json ./
COPY patches/ ./patches/
COPY web/package.json ./web/
COPY packages/ ./packages/
COPY ee/ ./ee/

RUN pnpm install --frozen-lockfile

# Prisma client first: `@langfuse/shared` type-checks against it, and `tsc` fails
# with "Module '@prisma/client' has no exported member 'Organization'" without
# it. The schema only needs a syntactically valid `DATABASE_URL` to generate —
# no connection is made, so the value is a placeholder.
RUN cd packages/shared \
    && DATABASE_URL="postgresql://unused:unused@127.0.0.1:5432/unused" \
       npx prisma generate --no-hints

# `@langfuse/shared` and `@langfuse/ee` are consumed through their build output
# (`main` is `./dist/src/index.js`), and `.dockerignore` deliberately keeps host
# build artifacts out of the context — so they must be compiled here. Without
# this, `next build` fails to resolve their exports (~210 "export not found"
# errors) because `dist/` does not exist in the image.
RUN pnpm --filter @langfuse/shared --filter @langfuse/ee run build

COPY web/ ./web/

# `DOCKER_BUILD=1` makes `web/src/env.mjs` skip schema validation: DATABASE_URL
# and friends are runtime configuration and are not known at image build time.
# `next build` is invoked directly rather than through the package script, which
# wraps it in `dotenv -e ../.env` and would fail on the absent file.
#
# `NEXT_IGNORE_BUILD_ERRORS=true` skips `next build`'s TypeScript step. The
# fork's web app has ~800 pre-existing type errors (the tRPC → Rust proxy is
# untyped at most call sites), so type-checking would fail the image build for
# reasons unrelated to it. Run `pnpm --filter web run typecheck` to see them.
RUN cd web && DOCKER_BUILD=1 NEXT_IGNORE_BUILD_ERRORS=true npx next build

# ============================================================================
# Stage 3 — runtime
# ============================================================================
FROM node:24-bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends tini curl ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=rust-build /src/langfuse-rs/target/release/server /usr/local/bin/langfuse-server

# `output: "standalone"` in a workspace emits `standalone/web/server.js` plus a
# pruned node_modules; static assets and public/ are not included and must be
# copied alongside.
COPY --from=web-build /src/web/.next/standalone ./
COPY --from=web-build /src/web/.next/static ./web/.next/static
COPY --from=web-build /src/web/public ./web/public

COPY docker/entrypoint.sh /usr/local/bin/entrypoint.sh

RUN chmod +x /usr/local/bin/entrypoint.sh \
    && useradd --create-home --uid 1001 langfuse \
    && chown -R langfuse:langfuse /app

USER langfuse

# 3000 — Next.js UI and the /api/public/* proxy.
# 8010 — the Rust API directly. Point OTLP exporters here to skip the Next.js
#        proxy hop; both ports serve the same ingest endpoint.
EXPOSE 3000 8010

# Checks the Rust API directly: /api/public/health is deliberately outside the
# API-key layer, so a probe needs no credentials.
HEALTHCHECK --interval=30s --timeout=5s --start-period=20s --retries=3 \
    CMD curl -fsS http://127.0.0.1:8010/api/public/health || exit 1

ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/entrypoint.sh"]
