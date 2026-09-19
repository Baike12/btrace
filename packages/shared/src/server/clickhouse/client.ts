// Stub for PG-only fork: no ClickHouse client needed.
// All ClickHouse operations are routed through pg.ts in repositories/.

export const convertDateToClickhouseDateTime = (date: Date): string => {
  return date.toISOString().replace("T", " ").replace("Z", "");
};

// PreferredClickhouseService type — stub for PG-only
export type PreferredClickhouseService = "default";
