// Stub: re-export from pg-sql, stub missing exports
export { orderByToPgSql as orderByToClickhouseSql } from "../pg-sql/orderby-factory";

import type { OrderByState } from "../../../interfaces/orderBy";
export function orderByToEntries(_orderBy: OrderByState): [string, string][] { return []; }
