// Stub: backward-compat for PG-only fork. Re-exports from pg.ts.
// Many repository files still import from "./clickhouse".
export {
  queryPg as queryClickhouse,
  executePg as commandClickhouse,
  queryPgStream as queryClickhouseStream,
  executePg as upsertClickhouse,
  parseClickhouseUTCDateTimeFormat,
  toPgTimestamp,
  pgCompliantRandomCharacters as clickhouseCompliantRandomCharacters,
} from "./pg";

export type ClickhouseQueryOpts = {
  query: string;
  params?: unknown[];
  tags?: Record<string, string>;
  clickhouseSettings?: Record<string, string>;
};
