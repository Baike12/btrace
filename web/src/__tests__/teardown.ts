/**
 * Per-project teardown for the vitest server projects.
 *
 * The pre-fork version closed the ClickHouse client pool and disconnected
 * Redis; both were removed when storage moved to Postgres, and the symbols it
 * imported (`ClickHouseClientManager`, `redis`) no longer exist on
 * `@langfuse/shared/src/server` — so the teardown threw
 * `TypeError: Cannot read properties of undefined` on every run.
 */
export default async function teardown() {
  const { logger } = await import("@langfuse/shared/src/server");
  logger.debug("Teardown complete");
}
