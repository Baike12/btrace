// Stub for PG-only fork: no ClickHouse client needed.
// All ClickHouse operations are routed through pg.ts in repositories/.
import type { ClickHouseClient } from "@clickhouse/client";

export const convertDateToClickhouseDateTime = (date: Date): string => {
  return date.toISOString().replace("T", " ").replace("Z", "");
};

// PreferredClickhouseService type — stub for PG-only
export type PreferredClickhouseService = "default";

// Stub ClickHouse client - not used in PG-only mode
export const clickhouseClient = null as unknown as ClickHouseClient;

// Re-export for backward compat
export { clickhouseClient as default };
