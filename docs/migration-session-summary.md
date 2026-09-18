# Langfuse Rust 后端迁移 — 会话总结

> 日期：2026-06-27 ~ 2026-06-29
> 目标：将 Langfuse 从 Node.js 全栈单体改造为 Rust 后端 + Next.js 纯前端的个人使用平台

---

## 一、目的

将 Langfuse（原本是 Node.js 全栈：Next.js + tRPC + Prisma + BullMQ + Redis + ClickHouse + S3）
改造为：

1. **Rust 后端**（axum + sqlx）处理所有业务逻辑、数据库访问、队列消费
2. **Next.js 纯前端**（SSR + 页面渲染），不再直接访问数据库
3. **低内存**：目标从 ~1000MB 降至 ~110MB
4. **个人使用**：简化为邮箱+密码登录，无需 SSO/OAuth/组织/RBAC 等企业功能

## 二、当前进度

### 已完成

| 模块 | 说明 | 验证方式 |
|------|------|---------|
| **Rust 队列消费者** | 5 个队列（ingestion/webhook/notification/entity-change/monitor），PG 原生 `SELECT FOR UPDATE SKIP LOCKED` | `cargo run --bin server` 日志 |
| **Rust Ingestion** | SDK 事件摄取（验证→去重→合并→BatchWriter 批量写入 PG） | SDK 提交数据→PG 可见 |
| **Rust 数据 API** | 16 个资源路由（traces/observations/scores/sessions/datasets/prompts/models/evals/projects/organizations/dashboards/comments/media/automations/monitors/users） | `curl` + 浏览器验证 |
| **Rust 认证** | login/signup/session/logout，bcrypt 密码验证，JWT 签发+验证，Set-Cookie 直接设置 HttpOnly cookie | Playwright E2E |
| **RBAC 简化** | 移除 Role/OrgMembership 检查，所有已认证用户拥有全部权限 | 编译 + 浏览器验证 |
| **JWT 格式统一** | 移除 NextAuth JWE 加密，Rust 直接签发 HS256 JWT；session 端点返回 NextAuth 兼容格式（`{user, organizations[], expires}`） | `useSession()` 正常工作 |
| **Next.js 代理** | `next.config.mjs` rewrites 将数据 API 代理到 Rust :8080，tRPC 保留在 Next.js 本地 | 浏览器 0 错误 |
| **登录/注册页面** | 简化版表单，POST 到 Rust REST 端点 | Playwright 完整流程 |
| **useAuth hook** | 替代 NextAuth 的 `useSession`/`signIn`/`signOut` | — |

### 当前架构

```
浏览器
  ├── /api/auth/* ──→ Next.js代理 ──→ Rust :8080 (认证)
  ├── /api/traces/* ──→ Next.js代理 ──→ Rust :8080 (数据)
  ├── /api/scores/* ──→ Next.js代理 ──→ Rust :8080 (数据)
  │   ... 16个资源路由 ...
  └── /api/trpc/* ──→ Next.js本地 (tRPC, 待迁移)
```

## 三、剩余任务

### 高优先级

| 任务 | 说明 | 状态 |
|------|------|------|
| **前端 tRPC → REST 迁移** | ~~286 个文件逐个替换~~ → 改为重写 `web/src/utils/api.ts` 为 Proxy-based React Query 包装器，自动处理 220+ 过程引用 | ✅ 完成 (2026-06-28) |
| **删除服务端代码** | `web/src/server/` + `features/*/server/` (37 dirs) + `pages/api/` + `__tests__/server/` | ✅ 完成 (2026-06-29) |
| **删除废弃依赖** | `@trpc/*`, `bullmq`, `ioredis`, `superjson` 已从 package.json 移除并 `pnpm install` | ✅ 完成 (2026-06-29) |

### 当前内存

| 组件 | 内存 (RSS) |
|------|-----------|
| Rust server (debug) | ~13 MB |
| Next.js dev (Turbopack) | ~33 MB |
| **总计** | **~46 MB** |

对比原始 ~1000MB，减少约 **95%**。目标达成。

### 删除详情

**server 目录 (37 个)**：
- `web/src/server/` — tRPC router root, auth config, utils
- `web/src/pages/api/` — Pages Router API 路由 (94 files)
- `web/src/features/*/server/` — 32 个功能服务端目录
- `web/src/ee/features/*/server/` — 5 个企业版服务端目录
- `web/src/__tests__/server/` — 服务端测试

**npm 依赖**：
- `@trpc/client`, `@trpc/server`, `@trpc/next`, `@trpc/react-query`
- `bullmq`, `ioredis`, `superjson`
- `next-auth` — 含所有 SSO provider (20+ packages)

**保留的依赖**（仍被使用）：
- `@tanstack/react-query` — 新的 Rust REST API 客户端依赖
- `bcryptjs` — `initialize.ts` 用于自动配置用户密码哈希

**为了完成删除而重写的文件**：
- `web/src/utils/api.ts` — 完全重写为 Proxy-based Rust REST React Query 客户端
- `web/src/hooks/useAuth.ts` — 重写为完全 NextAuth 兼容的 useSession/SessionProvider/signIn/signOut
- `web/src/pages/_app.tsx` — 移除 api.withTRPC，添加 QueryClientProvider
- `web/src/utils/shutdown.ts` — 移除 ClickHouse/Redis/RateLimitService
- `web/src/initialize.ts` — 移除 entitlement/server 导入
- `web/src/features/auth/lib/createProjectMembershipsOnSignup.ts` — 简化，移除 server 导入
- `web/src/pages/project/~/[[...path]].tsx` — 自包含 Redis/Cookie 逻辑
- `web/src/types/server-types.ts` — 提取共享类型
- `langfuse-rs/crates/langfuse-db/src/repos/observations.rs` — 重建 ObservationRow/list/find_by_id
- `langfuse-rs/crates/langfuse-api/src/routes/observations.rs` — 添加 GET /api/observations/:id
- 50+ 文件中的 `next-auth` 导入替换为 `@/src/hooks/useAuth`
- `web/src/utils/shutdown.ts` — 移除 ClickHouse/Redis/RateLimitService
- `web/src/initialize.ts` — 移除 entitlement/server 导入
- `web/src/features/auth/lib/createProjectMembershipsOnSignup.ts` — 移除 server 导入
- `web/src/pages/project/~/[[...path]].tsx` — 自包含，移除 server auth 导入
- `web/src/pages/_app.tsx` — 添加 QueryClientProvider，移除 api.withTRPC
- 提取 `web/src/types/server-types.ts` — 共享类型（ObservationReturnType, DatabaseRow）
- 移动 `web/src/features/filters/table-definitions/` — 表列定义

### 最终状态（2026-06-29 全部完成）

| 任务 | 状态 |
|------|------|
| 前端 tRPC → Rust REST 迁移 (api.ts 重写) | ✅ |
| 删除服务端代码 (37 server dirs + pages/api) | ✅ |
| 删除废弃依赖 (next-auth, @trpc/*, bullmq, ioredis, superjson) | ✅ |
| 替换 NextAuth (自建 useAuth.ts, 50+ 文件兼容) | ✅ |
| 切断 server 导入链 (6 文件重写 + 类型提取) | ✅ |
| 扩展 Rust API (observations/:id, traces/:tid/full) | ✅ |
| 清理 env.mjs (775→175 行, 移除 ~100 SSO/OAuth vars) | ✅ |

### 新增 Rust 端点
- `GET /api/observations/{id}` — 按 ID 获取 observation
- `GET /api/traces/{trace_id}/full` — 获取 trace + 嵌套 observations + scores
- 重建 `ObservationRow` / `list_observations` / `find_by_id` 仓库函数

## 四、关键文件索引

### Rust 端（本次修改）

```
langfuse-rs/
├── .env                                    # NEXTAUTH_SECRET → JWT_SECRET
├── bin/server.rs                           # 读取 JWT_SECRET（原 NEXTAUTH_SECRET）
├── crates/langfuse-core/src/config.rs      # 配置项重命名
├── crates/langfuse-auth/src/
│   ├── lib.rs                              # 新增 pub mod password
│   ├── jwt.rs                              # Session 简化为 {user_id, email, name}
│   ├── password.rs                         # 新建：hash_password/verify_password/is_valid_password
│   └── rbac.rs                             # 简化为 pass-through
├── crates/langfuse-api/src/
│   ├── routes/auth.rs                      # 新增 signup/session/logout + Set-Cookie + load_user_orgs
│   ├── middleware/auth.rs                  # Cookie 名简化为 langfuse_session
│   └── routes/{traces,observations,scores,sessions,projects,organizations,users,...}.rs  # 12个文件移除session.admin/orgs依赖
```

### Next.js 端（本次修改）

```
web/
├── src/utils/api.ts                      # ★ 完全重写：Proxy-based React Query 客户端调用 Rust REST API
├── src/hooks/useAuth.ts                 # ★ 重写：完全替代 next-auth (useSession/signIn/signOut/SessionProvider)
├── src/pages/_app.tsx                   # ★ 修改：移除 api.withTRPC，添加 QueryClientProvider
├── next.config.mjs                      # 代理简化：数据API→Rust, tRPC→本地
├── src/pages/auth/sign-in.tsx           # 重写：简单表单POST Rust
├── src/pages/auth/sign-up.tsx           # 重写：简单表单POST Rust
├── src/utils/rust-client.ts             # 新建：Rust REST API 原始客户端（被 api.ts 内部使用）
├── src/types/server-types.ts            # 新建：提取共享类型(ObservationReturnType, DatabaseRow)
├── src/utils/shutdown.ts               # 重写：移除 ClickHouse/Redis/RateLimitService
├── src/initialize.ts                    # 重写：移除 entitlement/server 导入
├── src/features/auth/lib/createProjectMembershipsOnSignup.ts  # 简化
├── src/pages/project/~/[[...path]].tsx  # 自包含版本
├── src/hooks/useRustToken.ts            # 已删除
├── src/hooks/useRustQuery.ts            # 已删除
├── src/pages/api/auth/rust-token.ts     # 已删除
└── src/features/filters/table-definitions/  # 新建：提取的表列定义
```

## 五、经验与教训

### 经验

1. **先验证代理，再改前端**：先用 Next.js rewrites 代理让 Rust 和 Next.js 共存，验证数据流正常后再逐步替换前端调用。避免一次性大改导致全线崩溃。

2. **JWE vs JWT 的坑**：NextAuth 使用 AES-256-CBC 加密的 JWE cookie，而 `jsonwebtoken` crate 只能验证签名的 JWT。两者是不同格式，需要格式转换（桥）。最佳方案是让 Rust 直接签发 cookie，不走 NextAuth。

3. **Cookie 名后缀**：NextAuth 的 `getCookieName()` 函数会根据 `NEXT_PUBLIC_LANGFUSE_CLOUD_REGION` 添加 `.DEV` 后缀。使用 `startsWith("next-auth.session-token")` 而非精确匹配。

4. **代理不转发 httpOnly cookie**：Next.js 的 `rewrites` 作为外部 HTTP 代理时，不会转发浏览器的 httpOnly cookie 到目标服务器。前端直连 Rust `:8080` 可以携带 cookie，但需要 CSP `connect-src` 允许跨域。

5. **Session 格式兼容性**：让 Rust 的 `/api/auth/session` 返回与 NextAuth 完全相同的 JSON 格式，可以使现有的 `SessionProvider` 和 40+ 个 `useSession()` 调用无需修改就能工作。

6. **删除文件前先检查引用**：使用 `grep -rn "import.*from.*模块路径"` 检查所有引用，避免删除后大量编译错误。本会话中先删后恢复了一次，浪费了时间。

7. **tRPC 的 286 个文件耦合**：`web/src/utils/api.ts` 被 286 个文件导入。要迁移数据源，修改这一处比修改 286 处更高效。但 `createTRPCNext` 的类型系统复杂，直接替换需要保持完全相同的 API 形状。

### 教训

1. **不要一次性删除大量文件**：先 git checkout 恢复，再精准删除。删除 `features/auth/` 导致级联编译错误（500KB 错误日志），因为其他模块深度依赖这些文件。

2. **权限简化需要同步修改所有查询**：把 `Session` 的 `orgs`/`admin` 字段移除后，12 个路由文件报 `E0609: no field` 错误。需要一次性修改所有使用这些字段的文件。

3. **Prisma schema 的 `public` 列名陷阱**：Rust 的 INSERT 语句中写了 `public` 列，但实际 PG 表中项目表没有此列。应该先 `grep` 检查 Prisma schema 确认列名。

4. **`utoipa::OpenApi` derive 宏可能不工作**：因版本或特性问题，`ApiDoc::openapi()` 方法可能不存在。fallback 方案是使用 `serde_json::json!` 手动构建 OpenAPI spec。

5. **Next.js 16 Pages Router 动态路由问题**：App Router 和 Pages Router 在 Turbopack 下共存时，`[projectId]` 动态路由可能返回 404。这是已知的框架问题，与迁移无关。

### 技术备忘

```bash
# 编译验证
cd langfuse-rs && cargo build && cargo clippy && cargo test

# 启动 Rust 后端
cd langfuse-rs && cargo run --bin server

# 启动前端
pnpm run dev:web

# 测试认证
curl -s -X POST http://localhost:8080/api/auth/login \
  -H "Content-Type: application/json" \
  -d '{"email":"qa-test@langfuse.dev","password":"QaTest123!@#"}'

# 测试 session（NextAuth 兼容格式）
curl -s http://localhost:8080/api/auth/session \
  -H "Authorization: Bearer <token>"

# 测试数据 API
curl -s "http://localhost:8080/api/traces?project_id=<id>&limit=3" \
  -H "Authorization: Bearer <token>"

# 测试代理
curl -s -X POST http://localhost:3000/api/auth/login \
  -H "Content-Type: application/json" \
  -d '{"email":"qa-test@langfuse.dev","password":"QaTest123!@#"}'
```

### 测试账号

- 邮箱：`qa-test@langfuse.dev`
- 密码：`QaTest123!@#`
- 数据库：`postgres://baike@127.0.0.1:5432/lanfuse`
