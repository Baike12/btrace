// PG-only stub: TraceUpsertQueue was a BullMQ sharded queue for trace-upsert events.
// In PG-only mode, eval scheduling is handled differently, so this queue is no-oped.
// The caller in IngestionService already handles null return gracefully.

export class TraceUpsertQueue {
  private static instance: TraceUpsertQueue | null = null;

  static getInstance(_opts?: {
    shardingKey?: string;
    shardName?: string;
  }): null {
    // PG-only: not initialized — returning null causes callers to skip
    // the trace-upsert eval scheduling path
    return null;
  }

  static getShardNames(): string[] {
    return [];
  }
}
