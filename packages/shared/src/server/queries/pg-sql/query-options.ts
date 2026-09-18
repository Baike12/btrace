/**
 * PG-compatible shouldSkipObservationsFinal.
 * In PG we use DISTINCT ON for dedup instead of FINAL.
 * Returns false by default (always deduplicate), as PG dedup is cheap.
 * OTel projects with immutable spans can skip dedup; for now, default to safe.
 */
export async function shouldSkipObservationsFinal(
  _projectId: string,
): Promise<boolean> {
  // In PG, DISTINCT ON dedup is efficient. Always apply it for safety.
  // OTel projects with immutable observations could skip it in the future.
  return false;
}
