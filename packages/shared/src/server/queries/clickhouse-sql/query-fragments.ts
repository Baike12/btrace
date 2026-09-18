// Stub for PG-only fork
export {
  eventsScoresAggregation,
  eventsSessionsAggregation,
  eventsTraceMetadata,
  eventsTracesAggregation,
  eventsTracesScoresAggregation,
} from "../pg-sql/query-fragments";

// eventsSessionScoresAggregation not in pg-sql, provide stub
export const eventsSessionScoresAggregation = () => ({ query: "SELECT 1", params: [] });
