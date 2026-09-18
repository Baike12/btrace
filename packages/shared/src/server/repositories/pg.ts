import { randomUUID } from "crypto";
import { prisma } from "../../db";
import { logger } from "../logger";
import { instrumentAsync } from "../instrumentation";
import { SpanKind } from "@opentelemetry/api";

/**
 * PG 查询引擎 — 替代 clickhouse.ts 中的所有 queryClickhouse/commandClickhouse/upsertClickhouse。
 */

export type PgQueryOpts = {
  query: string;
  params?: unknown[];
  tags?: Record<string, string>;
  // Backward-compat fields from ClickHouse query options
  table?: string;
  records?: unknown[];
  eventBodyMapper?: unknown;
  clickhouseConfigs?: unknown;
  preferredClickhouseService?: string;
};

// ============================================================
// queryPg — 替代 queryClickhouse<T>
// ============================================================
export async function queryPg<T>(opts: PgQueryOpts): Promise<T[]> {
  return await instrumentAsync(
    { name: "pg-query", spanKind: SpanKind.CLIENT },
    async (span) => {
      span.setAttribute("db.system", "postgresql");
      span.setAttribute("db.operation.name", "SELECT");
      span.setAttribute("db.query.text", opts.query.slice(0, 500));

      try {
        const result = await prisma.$queryRawUnsafe<T[]>(
          opts.query,
          ...(opts.params ?? []),
        );
        if (process.env.NODE_ENV === "development") {
          logger.info(`pg:query ${opts.query.slice(0, 200)}`);
        }
        return result;
      } catch (error) {
        logger.error("pg query error", {
          error: (error as Error).message,
          query: opts.query.slice(0, 200),
          tags: opts.tags,
        });
        throw error;
      }
    },
  );
}

// ============================================================
// executePg — 替代 commandClickhouse
// ============================================================
export async function executePg(opts: PgQueryOpts): Promise<void> {
  return await instrumentAsync(
    { name: "pg-command", spanKind: SpanKind.CLIENT },
    async (span) => {
      span.setAttribute("db.system", "postgresql");
      span.setAttribute("db.operation.name", "COMMAND");
      span.setAttribute("db.query.text", opts.query.slice(0, 500));

      try {
        await prisma.$executeRawUnsafe(
          opts.query,
          ...(opts.params ?? []),
        );
        if (process.env.NODE_ENV === "development") {
          logger.info(`pg:execute ${opts.query.slice(0, 200)}`);
        }
      } catch (error) {
        logger.error("pg execute error", {
          error: (error as Error).message,
          query: opts.query.slice(0, 200),
          tags: opts.tags,
        });
        throw error;
      }
    },
  );
}

// ============================================================
// queryPgStream — 替代 queryClickhouseStream
// 用 PG CURSOR 实现流式读取
// ============================================================
export async function* queryPgStream<T>(
  opts: PgQueryOpts & { fetchSize?: number },
): AsyncGenerator<T> {
  const cursorName = `cursor_${randomUUID().replace(/-/g, "_")}`;
  const fetchSize = opts.fetchSize ?? 1000;

  await prisma.$executeRawUnsafe("BEGIN");
  await prisma.$executeRawUnsafe(
    `DECLARE "${cursorName}" CURSOR FOR ${opts.query}`,
    ...(opts.params ?? []),
  );

  try {
    while (true) {
      const rows = await prisma.$queryRawUnsafe<T[]>(
        `FETCH ${fetchSize} FROM "${cursorName}"`,
      );
      if (rows.length === 0) break;
      for (const row of rows) {
        yield row;
      }
    }
  } finally {
    await prisma.$executeRawUnsafe(`CLOSE "${cursorName}"`);
    await prisma.$executeRawUnsafe("COMMIT");
  }
}

// ============================================================
// 工具函数
// ============================================================

/**
 * 将 JS Date 转为 PG timestamptz 兼容格式（ISO 8601 字符串）
 */
export function toPgTimestamp(date: Date): string {
  return date.toISOString();
}

/**
 * 将 ClickHouse DateTime64(3) 格式 "2024-05-23 18:33:41.602" 转为 Date
 * （兼容从旧 CH 读取的数据，PG 用 TIMESTAMPTZ 不会有这种格式）
 */
export function parseClickhouseUTCDateTimeFormat(dateStr: string): Date {
  return new Date(`${dateStr.replace(" ", "T")}Z`);
}

/**
 * 生成随机字符串（替代 clickhouseCompliantRandomCharacters）
 */
export function pgCompliantRandomCharacters(): string {
  const chars = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
  let result = "";
  const randomArray = new Uint8Array(5);
  crypto.getRandomValues(randomArray);
  randomArray.forEach((number) => {
    result += chars[number % chars.length];
  });
  return result;
}

// ============================================================
// Backward-compatibility aliases (ClickHouse → PG migration)
// These let existing code compile while SQL strings are gradually rewritten.
// ============================================================
export const queryClickhouse = queryPg;
export const queryClickhouseStream = queryPgStream;
export const commandClickhouse = executePg;
export const upsertClickhouse = executePg;

export const clickhouseCompliantRandomCharacters = pgCompliantRandomCharacters;
