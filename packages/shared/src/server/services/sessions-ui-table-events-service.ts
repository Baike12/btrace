// PG-only fork: Sessions events service stubs
// V4 events table (ClickHouse) features are not available in PG-only mode

export type SessionEventsDataReturnType = any;

export type SessionTraceFromEvents = {
  session_id: string;
  max_timestamp: string;
  min_timestamp: string;
  trace_ids: string[];
  user_ids: string[];
  trace_count: number;
  trace_tags: string[];
};

export const getSessionTracesFromEvents = async (_props: any): Promise<SessionTraceFromEvents[]> => {
  return [];
};

export const getSessionsTableCountFromEvents = async (_props: any): Promise<number> => {
  return 0;
};

export const getSessionsTableFromEvents = async (_props: any): Promise<any[]> => {
  return [];
};

export const getSessionsWithMetricsFromEvents = async (_props: any): Promise<any[]> => {
  return [];
};

export type FetchSessionsTableFromEventsProps = any;
