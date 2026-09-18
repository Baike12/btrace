// Stub for PG-only fork: no Redis/BullMQ queue
// OtelIngestion is handled via PG queue instead
export const OtelIngestionQueue = {
  getInstance: () => null,
};
