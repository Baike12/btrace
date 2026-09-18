---
name: langfuse-debug
description: |
  Debug the Langfuse PG-only fork frontend (Next.js + Rust backend).
  Use when the frontend UI is not behaving as expected — buttons not responding,
  API calls failing, pages not loading, or after making backend changes.
  Covers login credentials, how to start services, proxy architecture, and
  verified troubleshooting patterns.
---

# Langfuse Debug Skill

## Project Architecture

Browser (localhost:3000) -> Next.js (web/) frontend
  -> rewrites /api/* to Rust backend (localhost:8010)
  -> Rust (langfuse-rs/) axum + sqlx
  -> PostgreSQL (localhost:5432, db=lanfuse)

Auth: login sets langfuse_session cookie (HTTP-only JWT) -> all API calls use cookie auth via middleware.

## Start Services

Terminal 1 (Rust): cd langfuse-rs && cargo run --bin server  (port 8010)
Terminal 2 (Next.js): pnpm run dev:web  (port 3000)

If Rust crashes with step_by(0) overflow, set QUEUE_CONSUMER_COUNT=1 env var.

## Login

URL: http://localhost:3000/auth/sign-in
Email: a736513759b@gmail.com
Password: 12345678
Backup: qa-test@langfuse.dev / QaTest123!@#

## Existing Project

hoshi project under baike Organization:
- Project ID: 94d684fd-2b7c-40bd-8e33-3cdc432e8551
- Org ID: cmqjq16xn0001luy3ho55o7e7

## Key Files

- web/src/utils/api.ts — React Query client replacing tRPC. Proxy-based resource routing, REAL_RESOURCES map, custom procedure injections.
- web/src/hooks/useAuth.ts — Session management. cachedSession: undefined=loading, null=unauthenticated, object=authenticated.
- web/next.config.mjs — async rewrites() proxying /api/* to Rust :8010. Routes list determines which resources are proxied.
- langfuse-rs/crates/langfuse-api/src/app.rs — Rust router. Private routes have .layer(auth_layer).
- langfuse-rs/crates/langfuse-api/src/middleware/auth.rs — verify_session: checks Authorization header OR Cookie: langfuse_session.

## api.ts Architecture

REAL_RESOURCES maps resource names to their Rust endpoint configs.
Adding a new resource requires 4 changes:
1. Rust handler in routes/<name>.rs with pub fn router()
2. Register in app.rs: .nest("/api/<name>", routes::<name>::router())
3. Add to routes list in web/next.config.mjs
4. Inject custom procedures in api.ts via IIFE into resourceCache[name]

Proxy get trap fix (verified):
createResourceProxy get trap must check _target[procedure] before
falling through to createProcedureProxy, otherwise manually injected
procedures are silently ignored:
  if (procedure in _target) return _target[procedure];

Custom procedure injection pattern:
(function() {
  const cp = mutationProc("POST", "/api/X", "X", "create");
  const lp = queryProc("/api/X", "X", "all", (i) => ({...}));
  resourceCache["X"] = resourceCache["X"] || createResourceProxy("X");
  resourceCache["X"].create = cp;
  resourceCache["X"].all = lp;
})();

## Common Debugging

Button click does nothing:
- No request in Network tab -> api.ts returned a mock. Check REAL_RESOURCES.
- 404 -> Route not in Rust or not in Next.js rewrites.
- 401 -> Auth middleware rejected. Check cookie after login.

Spinner forever:
Check useAuth.ts — cachedSession must use tri-state: undefined | null | Session.
null=mock was the root cause before fix.

Quick API test:
curl -s -c /tmp/ck.txt -X POST http://127.0.0.1:3000/api/auth/login -H "Content-Type: application/json" -d '{"email":"a736513759b@gmail.com","password":"12345678"}'
curl -s -b /tmp/ck.txt http://127.0.0.1:3000/api/projects

## Database

psql -U baike -h 127.0.0.1 -d lanfuse
Key tables: users, organizations, organization_memberships, projects, project_memberships, api_keys.
