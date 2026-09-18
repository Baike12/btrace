/**
 * Shared types extracted from server code for frontend use.
 * Formerly in src/server/api/routers/traces.ts and src/server/api/services/sqlInterface.ts
 */
import { type Observation } from "@langfuse/shared";

export type ObservationReturnTypeWithMetadata = Omit<
  Observation,
  "input" | "output" | "metadata"
> & {
  traceId: string;
  metadata: string | null;
  traceName?: string | null;
  traceTags?: string[];
  userId?: string | null;
  sessionId?: string | null;
};

export type ObservationReturnType = Omit<
  ObservationReturnTypeWithMetadata,
  "metadata"
>;

export type DatabaseRow = {
  [key: string]: string | number | Date | Record<string, unknown> | null;
};

/** Compatibility type for AppRouter references */
export type AppRouter = any;
