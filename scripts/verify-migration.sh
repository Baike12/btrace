#!/bin/bash
# verify-migration.sh — Langfuse PG-Only 整体迁移验收脚本
# 用法: bash scripts/verify-migration.sh
set -e

PG_URL="postgres://baike@127.0.0.1:5432/langfuse"
PASS=0
FAIL=0

check() {
  local desc="$1"
  local cmd="$2"
  echo -n "  [$desc] ... "
  if eval "$cmd" > /dev/null 2>&1; then
    echo "PASS"
    PASS=$((PASS + 1))
  else
    echo "FAIL"
    FAIL=$((FAIL + 1))
  fi
}

check_reverse() {
  local desc="$1"
  local cmd="$2"
  echo -n "  [$desc] ... "
  if ! eval "$cmd" > /dev/null 2>&1; then
    echo "PASS"
    PASS=$((PASS + 1))
  else
    echo "FAIL"
    FAIL=$((FAIL + 1))
  fi
}

echo "========================================="
echo " Langfuse PG-Only 整体迁移验收"
echo "========================================="
echo ""

# ============ Section 1: 数据库 Schema ============
echo "--- 1. PG Schema ---"

check "langfuse 数据库存在" \
  "psql '${PG_URL}' -c 'SELECT 1'"

check "traces 表存在" \
  "psql '${PG_URL}' -c 'SELECT 1 FROM traces LIMIT 0'"

check "observations 表存在" \
  "psql '${PG_URL}' -c 'SELECT 1 FROM observations LIMIT 0'"

check "scores 表存在" \
  "psql '${PG_URL}' -c 'SELECT 1 FROM scores LIMIT 0'"

check "events 表存在" \
  "psql '${PG_URL}' -c 'SELECT 1 FROM events LIMIT 0'"

check "dataset_run_items 表存在" \
  "psql '${PG_URL}' -c 'SELECT 1 FROM dataset_run_items LIMIT 0'"

check "blob_storage_file_log 表存在" \
  "psql '${PG_URL}' -c 'SELECT 1 FROM blob_storage_file_log LIMIT 0'"

check "pg_jobs 表存在" \
  "psql '${PG_URL}' -c 'SELECT 1 FROM pg_jobs LIMIT 0'"

check "pg_seen_events 表存在" \
  "psql '${PG_URL}' -c 'SELECT 1 FROM pg_seen_events LIMIT 0'"

check "pg_rate_limits 表存在" \
  "psql '${PG_URL}' -c 'SELECT 1 FROM pg_rate_limits LIMIT 0'"

check "pg_cache 表存在" \
  "psql '${PG_URL}' -c 'SELECT 1 FROM pg_cache LIMIT 0'"

check "pg_project_flags 表存在" \
  "psql '${PG_URL}' -c 'SELECT 1 FROM pg_project_flags LIMIT 0'"

echo ""

# ============ Section 2: PG Queue 功能 ============
echo "--- 2. PG Queue 功能测试 ---"

check "pg_jobs 能正常入队" \
  "psql '${PG_URL}' -c \"INSERT INTO pg_jobs (job_id, queue_name, payload, state) VALUES ('verify-migration-test', 'verify-queue', '{\\\"test\\\": true}', 'waiting');\""

check "pg_jobs SKIP LOCKED 能正常出队" \
  "psql '${PG_URL}' -c \"
    WITH next_job AS (
      SELECT id FROM pg_jobs
      WHERE queue_name = 'verify-queue' AND state = 'waiting'
      LIMIT 1 FOR UPDATE SKIP LOCKED
    )
    UPDATE pg_jobs SET state = 'active' FROM next_job WHERE pg_jobs.id = next_job.id;\""

check "pg_jobs 清理测试数据" \
  "psql '${PG_URL}' -c \"DELETE FROM pg_jobs WHERE queue_name = 'verify-queue';\""

echo ""

# ============ Section 3: 新增基础设施代码存在 ============
echo "--- 3. 新增代码文件 ---"

SCRIPT_DIR="$(cd "$(dirname "$0")/.." && pwd)"

check "pg.ts 查询引擎存在" \
  "[ -f '${SCRIPT_DIR}/packages/shared/src/server/repositories/pg.ts' ]"

check "pg-sql/factory.ts 存在" \
  "[ -f '${SCRIPT_DIR}/packages/shared/src/server/queries/pg-sql/factory.ts' ]"

check "pg-sql/pg-filter.ts 存在" \
  "[ -f '${SCRIPT_DIR}/packages/shared/src/server/queries/pg-sql/pg-filter.ts' ]"

check "pg-sql/orderby-factory.ts 存在" \
  "[ -f '${SCRIPT_DIR}/packages/shared/src/server/queries/pg-sql/orderby-factory.ts' ]"

check "PgQueue 存在" \
  "[ -f '${SCRIPT_DIR}/packages/shared/src/server/pg-queue/PgQueue.ts' ]"

check "PgWorker 存在" \
  "[ -f '${SCRIPT_DIR}/worker/src/pg-queue/PgWorker.ts' ]"

check "PgWriter 存在" \
  "[ -f '${SCRIPT_DIR}/worker/src/services/PgWriter/index.ts' ]"

check "queueRegistry 存在" \
  "[ -f '${SCRIPT_DIR}/packages/shared/src/server/pg-queue/queueRegistry.ts' ]"

check "pgSeenEvents 存在" \
  "[ -f '${SCRIPT_DIR}/packages/shared/src/server/pg-queue/pgSeenEvents.ts' ]"

check "pgCache 存在" \
  "[ -f '${SCRIPT_DIR}/packages/shared/src/server/pg-queue/pgCache.ts' ]"

echo ""

# ============ Section 4: 关键代码无 ClickHouse/Redis 引用 ============
echo "--- 4. 关键数据路径无遗留引用 ---"

check "processEventBatch 无 S3/Redis 导入" \
  "! grep -n 'StorageService\|redis\|s3Slowdown' '${SCRIPT_DIR}/packages/shared/src/server/ingestion/processEventBatch.ts' | grep -v '//'"

check "ingestionQueue 使用 PgWriter" \
  "grep -n 'PgWriter' '${SCRIPT_DIR}/worker/src/queues/ingestionQueue.ts'"

check "IngestionService 使用 PgWriter" \
  "grep -n 'PgWriter' '${SCRIPT_DIR}/worker/src/services/IngestionService/index.ts'"

echo ""

# ============ Section 5: TypeScript 编译 ============
echo "--- 5. TypeScript 编译 ---"

check "packages/shared typecheck" \
  "cd '${SCRIPT_DIR}' && pnpm --filter @langfuse/shared run typecheck 2>&1"

check "worker typecheck" \
  "cd '${SCRIPT_DIR}' && pnpm --filter worker run typecheck 2>&1"

echo ""

# ============ Summary ============
echo "========================================="
echo " 验收结果: $PASS 通过, $FAIL 失败"
echo "========================================="

if [ "$FAIL" -gt 0 ]; then
  echo "FAIL: 迁移未完成 — 有 $FAIL 项检查失败"
  exit 1
else
  echo "PASS: 所有检查通过！PG-Only 核心基础设施就绪"
  exit 0
fi
