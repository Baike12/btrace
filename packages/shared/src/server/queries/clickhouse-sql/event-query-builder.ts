// Stub for PG-only fork: re-export what exists in pg-sql, stub the rest
export { CTEQueryBuilder } from "../pg-sql/event-query-builder";

// CH-specific builders — provide no-op stubs
export const EventsQueryBuilder = class {};
export const EventsAggQueryBuilder = class {};
export const EventsAggregationQueryBuilder = class {};
export const EventsSessionAggregationQueryBuilder = class {};
export const ExperimentsAggregationQueryBuilder = class {};
export function buildEventsFullTableSplitQuery(..._args: unknown[]): unknown { return null; }

export const OBSERVATION_FIELD_GROUP_FIELD_NAMES: string[] = [];

export type CTESchema = Record<string, unknown>;
export type CTEWithSchema = { cte: string; schema: CTESchema };
export type ExperimentsAggregationFieldSetName = string;
export type SessionEventsMetricsRow = Record<string, unknown>;
export type SplitQueryBuilder = unknown;
