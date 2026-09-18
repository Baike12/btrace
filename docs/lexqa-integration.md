# LexQA 对接指南

本仓库是 [Langfuse](https://langfuse.com) 的 PG-only 分支：所有存储统一在 PostgreSQL，
API Server 与队列消费者由单个 Rust 二进制承载，Next.js 退化为 UI + `/api/public/*` 反向代理。

本文说明 LexQA 如何仅依赖本仓库这一个 Langfuse 实例完成上报与查看，**替换掉原
`docker-compose.yml` 中的 Langfuse v3 自建栈**（`langfuse-db-init` / `langfuse-clickhouse` /
`langfuse-minio` / `langfuse-worker` / `langfuse-web` 共 5 个服务）。

> 本文只描述需要改动的内容，不包含对 LexQA 仓库的改动。LexQA 侧的修改由使用者自行完成。

---

## 1. 为什么可以只用一个容器

| 上游 Langfuse v3 自建栈 | 本分支 | 说明 |
|---|---|---|
| `langfuse-web`（Next.js） | 同一镜像内 `node web/server.js` | UI + `/api/public/*` 代理 |
| `langfuse-worker`（Node 队列消费者） | 同一镜像内 `langfuse-server` | `bin/server.rs` 已用 `tokio::spawn` 内联全部队列消费者 |
| `langfuse-clickhouse` | **不需要** | 事件直接写 PostgreSQL |
| `langfuse-minio` | **不需要** | 无 S3 事件/媒体存储 |
| 独立 Redis | **不需要** | 队列基于 `pg_jobs` 表（`SELECT ... FOR UPDATE SKIP LOCKED`） |
| `langfuse-db-init`（建库） | **保留** | 仍需在 LexQA-postgres 中存在 `langfuse` 数据库 |

进程模型：

```text
单容器 langfuse
├─ node  :3000  Next.js standalone（UI + /api/public/* → :8010）
└─ rust  :8010  axum API + 5 个队列消费者   ← 无独立 worker 进程
        │
        └── DATABASE_URL ──▶ LexQA-postgres 的 langfuse 库
```

`bin/worker.rs` 仍然存在，但**没有打进镜像**：它与 `server` 内联的消费者功能重复，
同时运行会让每个队列出现两批工作进程争抢同一批 `pg_jobs` 行。

---

## 2. 构建镜像

在**本仓库根目录**构建（构建上下文需要同时包含 `web/`、`packages/`、`ee/` 与 `langfuse-rs/`）：

```bash
docker build -t btrace:latest .
```

端口：

| 端口 | 用途 |
|---|---|
| `3000` | UI 与 `/api/public/*` 代理 |
| `8010` | Rust API 直连。**OTLP 上报建议走这里** |

> OTLP 建议直连 `8010`：`otlptracehttp` 发送的是二进制 protobuf，绕过 Next.js 的
> rewrite 代理可以避免多一跳，也避免代理层对非 JSON 请求体的处理差异。两个端口的
> ingest 行为完全一致。

---

## 3. 启动

```bash
docker run -d --name LexQA-btrace \
  --network LexQA-network \
  -p 3000:3000 -p 8010:8010 \
  -e DATABASE_URL="postgres://<user>:<pass>@postgres:5432/langfuse" \
  -e SALT="<openssl rand -base64 32>" \
  -e NEXTAUTH_SECRET="<openssl rand -base64 32>" \
  -e ENCRYPTION_KEY="<openssl rand -hex 32>" \
  -e LANGFUSE_INIT_ORG_ID=LexQA \
  -e LANGFUSE_INIT_ORG_NAME=LexQA \
  -e LANGFUSE_INIT_PROJECT_ID=LexQA \
  -e LANGFUSE_INIT_PROJECT_NAME=LexQA \
  -e LANGFUSE_INIT_PROJECT_PUBLIC_KEY=pk-lf-lexqa-init \
  -e LANGFUSE_INIT_PROJECT_SECRET_KEY=sk-lf-lexqa-init \
  -e LANGFUSE_INIT_ORG_MEMBER_EMAIL=qa-test@langfuse.dev \
  btrace:latest
```

`langfuse` 数据库需先存在（可沿用 LexQA 的 `langfuse-db-init` 服务，或手工
`CREATE DATABASE langfuse;`）。

### 环境变量

| 变量 | 默认 | 说明 |
|---|---|---|
| `DATABASE_URL` | — | **必填**。指向 LexQA-postgres 中的 `langfuse` 库 |
| `SALT` | `dev-salt` | API Key fast-hash 的盐。**必须与写入密钥时一致**，否则所有 key 校验失败 |
| `NEXTAUTH_SECRET` | — | UI 会话签名 |
| `ENCRYPTION_KEY` | — | 敏感配置加密 |
| `LANGFUSE_BIND_ADDRESS` | `0.0.0.0` | Rust API 监听地址。**不要用 `HOSTNAME`**（Docker 会把它设为容器 ID） |
| `PORT` | `8010` | Rust API 端口。**只传给 Rust 进程**：Next.js 也读 `PORT`，若整个容器共用会让 web 去抢 8010 而 `EADDRINUSE` |
| `WEB_PORT` | `3000` | Next.js 端口 |
| `RUST_API_URL` | `http://localhost:8010` | Next.js 代理目标 |
| `LANGFUSE_INIT_*` | — | 首次启动自动创建 org/project/API Key/用户 |
| `LANGFUSE_INIT_ORG_MEMBER_EMAIL` | — | 逗号分隔。把**已存在**账号加入新建 org 作 OWNER |
| `LANGFUSE_DISABLE_SIGNUP` | `false` | `true` 关闭 `POST /api/auth/signup`。账号由 `LANGFUSE_INIT_*` 下发时应打开 |
| `LANGFUSE_SECURE_COOKIES` | `false` | TLS 终止时设为 `true`，会话 cookie 带 `Secure`。**纯 HTTP 自建不要开**：浏览器会丢弃 `Secure` cookie，表现为"登录成功但一刷新就退出" |
| `LANGFUSE_CORS_ALLOWED_ORIGINS` | 空（不允许跨域） | 逗号分隔的允许源。缺省即关闭：浏览器只访问 Next.js 那个源 |

`LANGFUSE_INIT_*` 是**幂等**的：已存在的 public key 会被原样保留。因此重启容器不会
轮换密钥，LexQA 侧配置好的 `sk` 不会失效。

### 浏览器核对前的必要两步

**第一步：要有一个能登录的账号。** 控制台要求登录（见 §8.1）。全新实例上没有任何账号，
用 `LANGFUSE_INIT_USER_*` 创建一个（幂等：账号已存在则跳过，不会改密码）：

```bash
-e LANGFUSE_INIT_USER_EMAIL="admin@example.com" \
-e LANGFUSE_INIT_USER_NAME="Admin" \
-e LANGFUSE_INIT_USER_PASSWORD="change-me-please" \
-e LANGFUSE_INIT_ORG_ID=LexQA \
-e LANGFUSE_INIT_PROJECT_ID=LexQA
```

若实例里已有账号（例如本地库里的 `qa-test@langfuse.dev`），跳过这步，直接用它的口令登录；
忘了口令就在 `users` 表里换一个 bcrypt 哈希。

**第二步：把账号挂到 `LANGFUSE_INIT_ORG_*` 建出来的组织上。**
`LANGFUSE_INIT_ORG_*` 新建的组织**没有任何成员**。若不做这一步，登录后项目列表为空、
上报的数据在浏览器里看不到：

```bash
-e LANGFUSE_INIT_ORG_MEMBER_EMAIL="qa-test@langfuse.dev"
```

`LANGFUSE_INIT_USER_*` 自己创建的用户已经同时加入该 org 与 project，无需再列在
`LANGFUSE_INIT_ORG_MEMBER_EMAIL` 里。

---

## 4. LexQA 侧改动

### 4.1 `docker-compose.yml`

删除 `langfuse-db-init` 之外的全部 Langfuse 服务（`langfuse-clickhouse`、
`langfuse-minio`、`langfuse-worker`、`langfuse-web`），替换为一个 `langfuse` 服务：

```yaml
  langfuse:
    image: btrace:latest
    container_name: LexQA-btrace
    restart: unless-stopped
    depends_on:
      langfuse-db-init:
        condition: service_completed_successfully
    ports:
      - "${LANGFUSE_WEB_PORT:-3000}:3000"
      - "${LANGFUSE_API_PORT:-8010}:8010"
    environment:
      DATABASE_URL: "postgresql://${DB_USER}:${DB_PASSWORD}@postgres:5432/${LANGFUSE_DB_NAME:-langfuse}"
      SALT: ${LANGFUSE_SALT:-lexqa-langfuse-dev-salt-change-me}
      NEXTAUTH_SECRET: ${LANGFUSE_NEXTAUTH_SECRET:-lexqa-langfuse-dev-nextauth-secret-change-me}
      ENCRYPTION_KEY: ${LANGFUSE_ENCRYPTION_KEY:-0000000000000000000000000000000000000000000000000000000000000000}
      LANGFUSE_INIT_ORG_ID: ${LANGFUSE_INIT_ORG_ID:-LexQA}
      LANGFUSE_INIT_ORG_NAME: ${LANGFUSE_INIT_ORG_NAME:-LexQA}
      LANGFUSE_INIT_PROJECT_ID: ${LANGFUSE_INIT_PROJECT_ID:-LexQA}
      LANGFUSE_INIT_PROJECT_NAME: ${LANGFUSE_INIT_PROJECT_NAME:-LexQA}
      LANGFUSE_INIT_PROJECT_PUBLIC_KEY: ${LANGFUSE_INIT_PROJECT_PUBLIC_KEY:-pk-lf-lexqa-init}
      LANGFUSE_INIT_PROJECT_SECRET_KEY: ${LANGFUSE_INIT_PROJECT_SECRET_KEY:-sk-lf-lexqa-init}
      LANGFUSE_INIT_ORG_MEMBER_EMAIL: ${LANGFUSE_INIT_ORG_MEMBER_EMAIL:-}
    networks:
      - LexQA-network
    profiles:
      - langfuse
      - full
```

### 4.2 `.env`

```bash
LANGFUSE_PUBLIC_KEY=pk-lf-lexqa-init
LANGFUSE_SECRET_KEY=sk-lf-lexqa-init
# 容器间通信用服务名；直连 Rust API 可省掉 Next.js 代理的一跳
LANGFUSE_HOST=http://langfuse:8010
# 若希望与 UI 同源（走 Next.js 代理），改用：
# LANGFUSE_HOST=http://langfuse:3000
```

LexQA 的 `.env.example` 中 `pk-lf-lexqa-init` / `sk-lf-lexqa-init` 已是预置值，
与上面的 `LANGFUSE_INIT_PROJECT_*` 成套，无需另行生成。

> **关于 `LANGFUSE_*` 客户端调优变量**（`LANGFUSE_FLUSH_AT`、`LANGFUSE_SAMPLE_RATE`、
> `LANGFUSE_QUEUE_SIZE` 等）仍然生效，它们完全在 LexQA 进程内工作，与本仓库无关。

### 4.3 不需要改的部分

- `internal/tracing/langfuse/*` 全部保持不变：exporter 仍是 OTLP/HTTP + Basic Auth +
  `x-langfuse-ingestion-version: 4`。
- 上报的 span 属性名、`usage_details` 键名无需调整。

---

## 5. 验证

```bash
# 1. 健康检查（无需凭证）
curl -s -o /dev/null -w '%{http_code}\n' http://localhost:8010/api/public/health
# 期望 200

# 2. 用 LexQA 的凭证发一条 OTLP 报文（这里用 JSON 便于调试；真实客户端发 protobuf）
curl -s -u "pk-lf-lexqa-init:sk-lf-lexqa-init" \
  -H 'Content-Type: application/json' \
  -H 'x-langfuse-ingestion-version: 4' \
  -d @payload.json \
  http://localhost:8010/api/public/otel/v1/traces
# 期望 200 + 空 JSON 对象 {}

# 3. 触发一次 LexQA 知识检索，等待 3 秒后在控制台查看
#    http://localhost:3000 → 用 §3 的账号登录 → Traces
```

控制台需要登录（见 §8.1），未登录时任何私有 `/api/*` 都返回 401：

```bash
curl -s -o /dev/null -w '%{http_code}\n' http://localhost:8010/api/projects
# 期望 401
```

OTLP/JSON 请求体中 `traceId` / `spanId` / `parentSpanId` 支持三种写法：hex 字符串、
base64 字符串、字节数组。protobuf 编码不受影响。

---

## 6. LexQA 文档承诺的能力对照

| LexQA `docs/Langfuse集成.md` 的承诺 | 本仓库支持 | 落库位置 |
|---|---|---|
| chat / embedding / rerank / VLM / ASR 五类调用上报 | ✅ | `observations`，`type=GENERATION` |
| 端到端 trace + asynq SPAN 树 | ✅ | `traces` + `observations.parent_observation_id` |
| 跨进程 trace 透传（W3C trace id 复用） | ✅ | `traces.id = hex(traceId)` |
| 流式首 token 延迟（TTFT） | ✅ | `observations.completion_start_time` |
| Input / Output / Total tokens | ✅ | `prompt_tokens` / `completion_tokens` / `total_tokens` |
| **Cache Read / Write / Miss tokens** | ✅ | `observations.usage_details`（jsonb） |
| `userId`（空间 ID） | ✅ | `traces.user_id`（`user.id` 属性） |
| `sessionId` | ✅ | `traces.session_id`（`session.id` 属性）+ `trace_sessions` |
| Sessions 视图聚合 | ✅ | `trace_sessions` + `GET /api/public/sessions` |
| 在 Settings → Models 配单价后自动核算费用 | ✅ | `calculated_*_cost` / `cost_details` |
| 非 token 计量（ASR 按秒） | ✅ | 档位无 token 单价时按 `models.total_price × 用量`，见 §6.2 |
| 一条命令拉起自建栈 | ✅ | 单容器，见第 2、3 节 |
| 不配置 `LANGFUSE_*` 时零开销 | ✅（LexQA 侧行为） | — |

### 6.1 一次上报的落库形状

LexQA 的根 span 带 `langfuse.observation.type=trace`，**只生成 `traces` 行**；其余 span
生成 `observations` 行。这与 Langfuse v4 语义一致，因此控制台的 trace 树不会出现重复
的根节点。

```text
traces (id = hex(traceId))
├── observations: asynq.document:process        (SPAN)
│     └── chat.completion.stream                (GENERATION, usage_details 含 cache 分项)
└── observations: rerank                        (SPAN, level=ERROR)
```

### 6.2 计价规则

单价单位是**每个 token**（与库内预置价一致：gpt-4o `input=0.0000025` 即 $2.5 / 百万）。
计价复刻官方 `IngestionService.calculateUsageCosts`，产出两张表：

```text
usage_details (jsonb)                          cost_details (jsonb)
{ input: 1200,  output: 300,                   { input: 0.00288,
  total: 1500,                                   output: 0.00288,
  cache_read_input_tokens: 800,                  cache_read_input_tokens: 0.000192,
  cache_creation_input_tokens: 100,              cache_creation_input_tokens: 0.0003,
  cache_miss_input_tokens: 300 }                 total: 0.006252 }
```

三件必须记住的事：

- **`cost_details` 的键就是 usage type**，不是固定的 input/output 两档。这样
  `cache_read_input_tokens` / `cache_creation_input_tokens` 各自成行、各自按单价计费，
  不会被混算成普通 input。UI 的「Cost breakdown」据此逐项展开。
- **值是 JSON number，且一定带 `total` 键**。前端 `CostDetails` 只保留
  `typeof value === "number"` 的条目；traces/observations 表格的总费用列读的是
  `cost_details['total']`。写成字符串会被静默丢弃。
- **`calculated_*_cost` 是 `cost_details` 的归约结果**，与读路径
  （`reduceUsageOrCostDetails`）用同一条规则算出，两者不会打架：

  ```text
  calculated_input_cost  = Σ cost_details[k] where k.startsWith("input")
  calculated_output_cost = Σ cost_details[k] where k.startsWith("output")
  calculated_total_cost  = cost_details.total
  ```

  注意是 `startsWith` 而非 `includes`：上面例子里 `calculated_input_cost = 0.00288`
  只含 `input` 一档，两个 `cache_*_input_tokens` 的键不以 `input` 开头，因此不计入。
  汇总口径看 `calculated_total_cost`。

其余规则：

- **只选中一个 pricing tier**：非 default 档按 `priority` 升序取第一个条件全满足的，否则
  落到 default 档；default 档之外的空条件视为不匹配。这样互斥的分档价不会互相叠加。
- 未配置单价的 usage type 不计费（`cache_miss_input_tokens` 在预置价中无对应项）。
- 客户端若在 span 上带了 `langfuse.observation.cost_details`，**以它为准，服务端不再计算**
  （官方的「用户给了费用就不再算」规则）。
- `unit` 非 token（如 `SECONDS`）：若该档没有任何匹配到单价的 usage type，会退回
  `models.total_price × usage_details.total`。这条兜底只为没迁移到 `prices` 表的旧模型
  （例如预置的 `whisper-large-v3`）保留，正常配置的档位不会被它覆盖。此时
  `calculated_input_cost` / `calculated_output_cost` 为 NULL（按秒计费没有 input/output
  之分），只有 `calculated_total_cost` 有值——与官方一致。

> **口径提示**：`usage_details` 中每个键都独立计费，不做相互扣减。Anthropic 的
> `input` 不含缓存部分，OpenAI 的 `prompt_tokens` 含——两种约定并存，服务端无法区分。
> 只配置你希望计费的那几档单价即可控制口径。

---

## 7. 公开 API 支持范围

| 端点 | 状态 |
|---|---|
| `POST /api/public/ingestion` | ✅ 同步写入 |
| `POST /api/public/otel/v1/traces` | ✅ protobuf / JSON / gzip |
| `GET /api/public/traces` | ✅ 分页 + `userId`/`sessionId`/`name`/`tags`/`version`/`release`/时间范围/`orderBy` |
| `GET /api/public/traces/{traceId}` | ✅ 内嵌 observations 与 scores |
| `GET /api/public/sessions`、`/sessions/{id}` | ✅ |
| `GET /api/public/metrics/daily` | ✅ 按天 + 按模型/单位的用量与费用（默认近 7 天，可用 `fromTimestamp`/`toTimestamp` 指定） |
| `GET`/`POST /api/public/models`、`GET`/`DELETE /api/public/models/{id}` | ✅ 含分项单价（`pricingTiers`） |
| `GET /api/public/observations`、`/api/public/scores` | ⛔ 返回 **404**（本次收敛范围外） |
| `GET /api/public/v2/metrics` | ⛔ 返回 **501** |
| `GET /api/public/v2/observations` | ⛔ 返回 **501** |

v2 端点返回 `501 Not Implemented` 并在 `message` 中说明替代方案，而不是 `404`——避免与
路径写错混淆。observations / scores 两个 v1 列表端点本分支未实现（返回 404）；LexQA
所需的数据经 `traces` / `traces/{id}` / `sessions` 已可完整获取，控制台自身也走内部
端点（见 §7.1）。

- **v2 metrics** 是一个通用 OLAP 查询引擎（3 个 view、约 20 个维度、约 15 个度量、
  11 种聚合含 p50–p99 与直方图、过滤算子矩阵、时间粒度、排序），且响应契约是开放的
  `list<map<string, unknown>>`。部分实现会返回「看起来合理但错误」的数字。
  按天/按模型的用量与费用请用 `metrics/daily`。
- **v2 observations** 是游标分页 + 10 个字段组 + 元数据截断的接口。observations
  目前可通过 `GET /api/public/traces/{traceId}` 获取。

### 7.1 控制台自身依赖的内部端点

这些不是公开 API（前缀不是 `/api/public`），但控制台的 Traces / Sessions / Settings
页面靠它们取数，部署后若缺失会表现为「页面空白」而不是报错。

**它们要求会话 cookie**（见 §8.1），因此用 curl 单独调它们之前要先登录拿 cookie：

```bash
curl -s -c /tmp/lf.txt -X POST http://localhost:8010/api/auth/login \
  -H 'content-type: application/json' \
  -d '{"email":"admin@example.com","password":"<口令>"}'
curl -s -b /tmp/lf.txt "http://localhost:8010/api/sessions/hasAny?project_id=LexQA"
```

每个带 `project_id` 的端点还会校验会话用户属于该项目的组织；不属于时返回 `404`。

| 端点 | 用途 |
|---|---|
| `GET /api/traces/{traceId}/full` | trace 详情页的 span 树、分项 usage / 费用 |
| `GET /api/traces/metrics?trace_ids=` | Traces 列表的 Observation Levels / Latency / Tokens / Total Cost 列（**扁平**字段：`promptTokens`、`errorCount`、`calculatedTotalCost`，表格逐字段重建行数据） |
| `GET /api/observations?trace_id=&type=&cursor=` | observation 明细（`type` 与 `cursor` 在 SQL 层生效） |
| `GET /api/sessions/hasAny` | Sessions 页在「空态」与「表格」之间二选一 |
| `GET /api/sessions/metrics?session_ids=` | Sessions 表的时长 / trace 数 / 费用 / token 列 |
| `GET /api/models?project_id=&page=&limit=` | Settings → Model Definitions 的列表，含 `pricingTiers` 与 `totalCount` |

验收时要看的是 `cost_details` 的**数值**（不是 key 名）：控制台的 Cost breakdown 会直接
列出 `input` / `output` / `cache_read_input_tokens` / `cache_creation_input_tokens` 四行
加 `total`。

---

## 8. 已知限制

### 8.1 控制台需要登录（8010 不再是"无认证"端口）

控制台自身的私有路由（`/api/*`，前缀不是 `/api/public`）**现在要求会话**：

```console
$ curl -s -o /dev/null -w '%{http_code}\n' http://<host>:8010/api/projects
401                      # 未带凭证
$ curl -s -X POST http://<host>:8010/api/auth/login \
    -H 'content-type: application/json' \
    -d '{"email":"qa-test@langfuse.dev","password":"<口令>"}'
200                      # 返回 Set-Cookie: langfuse_session=<JWT>
```

- 登录校验 `users.password`（bcrypt），签发用 `JWT_SECRET` 签名的 JWT，写入
  `HttpOnly` cookie。`middleware/auth.rs` 校验它并注入 `Session`。
- 每个带 `project_id` 的 handler **都会再校验会话用户属于该项目的组织**；不属于时返回
  `404`（而不是 403——403 会确认项目存在，可被用来枚举 project id）。
- 公开 API（`/api/public/*`）用的是 **API Key**，与上面这套互不相通：API Key 不是会话，
  会话也不是 API Key。LexQA 的上报与读取**不受影响，仍然只用 `pk`/`sk`**。
- 跨域访问默认关闭。浏览器只访问 Next.js 那个源，由 Next.js 在服务端转发 `/api/*`，
  因此不需要 CORS。确需跨域时显式列出源：
  `LANGFUSE_CORS_ALLOWED_ORIGINS="https://langfuse.example.com"`。

### 8.2 其他

- 镜像构建会**跳过 Next.js 的类型检查**（`NEXT_IGNORE_BUILD_ERRORS=true`）。本分支的
  web 应用有约 800 个既有类型错误（tRPC 被换成无类型代理后，多数调用点丢了类型），
  否则镜像构建会因与本适配无关的原因失败。需要看清单时跑
  `pnpm --filter web run typecheck`。
- `usage_details` 的键在页面展示时保持原样（`cache_read_input_tokens`），不会被前端
  改写成 camelCase——这些键是数据而非字段名，改名后既对不上 `prices` 也对不上客户端
  上报的原名。
- Trace 详情页的 Usage breakdown 里「Input usage」区头是**包含** `input` 子串的键求和，
  于是 `cache_read_input_tokens` 等会被一并加进去，看起来比 `input` 本身大。这是官方
  `BreakdownTooltip` 的行为，权威数字看 `Total usage`（即 `usage_details.total`）与
  trace 树上的 `1,200 → 300 (∑ 1,500)` 徽章。

---

## 9. 排障

| 现象 | 排查 |
|---|---|
| 容器起不来，日志停在 `Connecting to database` | `DATABASE_URL` 不可达，或 `langfuse` 库不存在 |
| 日志出现 `failed to lookup address` 或绑定失败 | 误把 `HOSTNAME` 当监听地址。用 `LANGFUSE_BIND_ADDRESS` |
| 上报返回 401 且 `message` 为 `Invalid API key` | `SALT` 与创建密钥时不一致；或 `pk`/`sk` 不配对 |
| 上报返回 400 `Invalid OTLP/protobuf payload` | `Content-Type` 与请求体编码不匹配 |
| 上报 200 但控制台看不到 | 未设置 `LANGFUSE_INIT_ORG_MEMBER_EMAIL`，或登录账号不属于该 org |
| token 有值但费用为空 | 该 model 未匹配到 `models.match_pattern`，或未配置单价 |
| cache 分项为 0 | 客户端未上报 `usage_details` 中的 cache 键，或该 usage type 无单价 |
| 一条 span 缺失 | 正常：OTLP 为部分成功语义。响应体的 `partialSuccess.errorMessage` 给出原因 |

服务端日志：`docker logs LexQA-btrace`。Rust 侧默认 `RUST_LOG=info,langfuse=debug`。
