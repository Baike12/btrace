/**
 * Types for the v4 events tables.
 *
 * `EventBatchIOOutput` is one row of `api.events.batchIO`: the observations,
 * traces and sessions tables join it onto their rows by id to fill in the
 * input/output column, and `useParsedObservation` reads the I/O off the merged
 * row. The original definition lived in
 * `features/events/server/eventsRouter.ts`, which this fork removed when the
 * backend moved to Rust; the payload is still only consumed through that join,
 * so the fields stay `unknown` until something needs a narrower type.
 */
export type EventBatchIOOutput = {
  id: string;
  input?: unknown;
  output?: unknown;
  metadata?: unknown;
};
