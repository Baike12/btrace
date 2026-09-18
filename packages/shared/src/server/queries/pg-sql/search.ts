/**
 * PG-compatible search condition builder.
 * Replaces clickhouseSearchCondition with positional $N params.
 */
export function pgSearchCondition(opts: {
  query?: string | null;
  tablePrefix?: string;
  searchType?: string[];
}): { query: string; params: unknown[] } {
  if (!opts.query) return { query: "", params: [] };
  const prefix = opts.tablePrefix ? `"${opts.tablePrefix}".` : "";
  const likePattern = `'%' || $1 || '%'`;
  return {
    query: `AND (${prefix}"id" ILIKE ${likePattern} OR ${prefix}"name" ILIKE ${likePattern})`,
    params: [opts.query],
  };
}
