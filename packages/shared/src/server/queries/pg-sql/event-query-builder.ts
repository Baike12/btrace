// PG-only: stub for ClickHouse event-query-builder
// Used by events.ts for building event queries

export const CTEQueryBuilder = class {
  ctes: string[] = [];
  addCte(_name: string, _query: string) { return this; }
  build() { return { query: "SELECT 1", params: [] }; }
};
