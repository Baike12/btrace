// PG-only: Redis cache stubs for eval job configuration optimization.
// No Redis available — always return false (cache miss) for reads, no-op for writes.

/** Cache types for different eval job configuration targets */
type EvalConfigCacheType = "traceBased" | "eventBased";

/**
 * Check if a project has no eval configurations cached.
 * PG-only: always returns false (cache miss) so caller falls back to DB query.
 */
export const hasNoEvalConfigsCache = async (
  _projectId: string,
  _cacheType: EvalConfigCacheType,
): Promise<boolean> => false;

/**
 * Cache that a project has no executable eval configurations.
 * PG-only: no-op (no Redis).
 */
export const setNoEvalConfigsCache = async (
  _projectId: string,
  _cacheType: EvalConfigCacheType,
): Promise<void> => { /* no-op */ };

/**
 * Clear the "no eval configs" cache for a project.
 * PG-only: no-op (no Redis).
 */
export const clearNoEvalConfigsCache = async (
  _projectId: string,
  _cacheType: EvalConfigCacheType,
): Promise<void> => { /* no-op */ };

export const invalidateProjectEvalConfigCaches = (
  _projectId: string,
): Promise<[void, void]> =>
  Promise.all([
    clearNoEvalConfigsCache(_projectId, "traceBased"),
    clearNoEvalConfigsCache(_projectId, "eventBased"),
  ]);
