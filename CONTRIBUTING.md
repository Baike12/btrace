# Contributing

btrace is a fork of [Langfuse](https://github.com/langfuse/langfuse) with a PostgreSQL-only
storage layer and a Rust backend. It is a separate project from upstream and does not use
Langfuse's contribution channels.

## Working in this repo

[`AGENTS.md`](AGENTS.md) is the entry point. It holds the architecture rules, the core
commands, the verification matrix per package, and a known-issues list that explains which
failing checks are pre-existing.

- Commit messages follow Conventional Commits (`fix:`, `feat:`, `docs:`, `refactor:`, …).
- Before signoff: `pnpm run lint`, `pnpm run typecheck`, and
  `cd langfuse-rs && cargo test --workspace && cargo clippy`.
- A `.husky/pre-commit` hook rejects staged build output and backup artifacts;
  `.husky/pre-push` asks for confirmation before pushing to `main`.
- Never commit secrets or credentials. Keep `.env*.example` files in sync with the
  environment variables the code actually reads.

## Upstream

This repository does not track Langfuse's roadmap, issue tracker, or release process.
Bugs and feature requests for the upstream project belong in the
[Langfuse repository](https://github.com/langfuse/langfuse).
