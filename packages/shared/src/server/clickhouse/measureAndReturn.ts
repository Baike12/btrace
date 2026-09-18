// Stub for PG-only fork
// measureAndReturn was used to time ClickHouse queries with Datadog metrics.
// In PG-only mode, we just execute the function and return its result.

export async function measureAndReturn<T>(
  _opts: { tags?: Record<string, string>; startTime?: number },
  fn: () => Promise<T>,
): Promise<T> {
  return fn();
}
