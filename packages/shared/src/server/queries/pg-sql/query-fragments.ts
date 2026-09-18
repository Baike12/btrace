// PG-only: stub for ClickHouse query-fragments
// Used by events.ts for event aggregation CTEs

export const eventsScoresAggregation = () => ({ query: "SELECT 1", params: [] });
export const eventsSessionsAggregation = () => ({ query: "SELECT 1", params: [] });
export const eventsTraceMetadata = () => ({ query: "SELECT 1", params: [] });
export const eventsTracesAggregation = () => ({ query: "SELECT 1", params: [] });
export const eventsTracesScoresAggregation = () => ({ query: "SELECT 1", params: [] });
export const eventsObservationsAggregation = () => ({ query: "SELECT 1", params: [] });
export const eventsExperiments = () => ({ query: "SELECT 1", params: [] });
export const eventsExperimentTraceIds = () => ({ query: "SELECT 1", params: [] });
