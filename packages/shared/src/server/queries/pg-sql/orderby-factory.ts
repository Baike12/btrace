import { OrderByState } from "../../../interfaces/orderBy";

/**
 * orderByToPgSql — PG 版本
 * 将 OrderByState 转换为 PG ORDER BY 子句。
 */
export function orderByToPgSql(
  orderBy: OrderByState | null | undefined,
  tablePrefix?: string,
): string {
  if (!orderBy) return "";

  const col = tablePrefix
    ? `"${tablePrefix}"."${orderBy.column}"`
    : `"${orderBy.column}"`;
  return `ORDER BY ${col} ${orderBy.order === "ASC" ? "ASC" : "DESC"}`;
}

/**
 * 获取默认排序：按 timestamp 降序
 */
export function getDefaultOrderBy(tablePrefix?: string): string {
  const col = tablePrefix ? `"${tablePrefix}"."timestamp"` : `"timestamp"`;
  return `ORDER BY ${col} DESC`;
}
