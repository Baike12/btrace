// @ts-nocheck
import { randomUUID } from "crypto";
import { z } from "zod";

import { env } from "../../env";
import {
  InvalidRequestError,
  LangfuseNotFoundError,
  UnauthorizedError,
} from "../../errors";
import { AuthHeaderValidVerificationResultIngestion } from "../auth/types";
// PG-only: entity type classification (formerly from ClickHouse)
export type IngestionEntityTypes = "trace" | "observation" | "score" | "dataset_run_item";
export const getClickhouseEntityType = (type: string): IngestionEntityTypes => {
  if (type === "trace-create") return "trace";
  if (type === "score-create") return "score";
  if (type === "dataset-run-item-create") return "dataset_run_item";
  // All other event types (span-create, generation-create, agent-create,
  // tool-create, chain-create, observation-create, etc.) map to "observation"
  return "observation";
};
import {
  getCurrentSpan,
  instrumentAsync,
  recordDistribution,
  recordIncrement,
} from "../instrumentation";
import { logger } from "../logger";
import { QueueJobs } from "../queues";
import {
  eventTypes,
  createIngestionEventSchema,
  IngestionEventType,
} from "./types";
import { getPgQueue } from "../pg-queue/queueRegistry";
import { isTraceIdInSample } from "./sampling";


/**
 * Get the delay for the event based on the event type. Uses delay if set, 0 if current UTC timestamp is not between
 * 23:45 and 00:15, and env.LANGFUSE_INGESTION_QUEUE_DELAY_MS otherwise.
 * We need the delay around date boundaries to avoid duplicates for out-of-order processing of events.
 * @param delay - Delay overwrite. Used if non-null.
 */
const getDelay = (delay: number | null, source: "api" | "otel") => {
  if (delay !== null) {
    return delay;
  }
  const now = new Date();
  const hours = now.getUTCHours();
  const minutes = now.getUTCMinutes();

  if ((hours === 23 && minutes >= 45) || (hours === 0 && minutes <= 15)) {
    return env.LANGFUSE_INGESTION_QUEUE_DELAY_MS;
  }

  if (source === "otel") {
    return 0;
  }

  // Use 5s here to avoid duplicate processing on the worker. If the ingestion delay is set to a lower value,
  // we use this instead.
  // Values should be revisited based on a cost/performance trade-off.
  return Math.min(5000, env.LANGFUSE_INGESTION_QUEUE_DELAY_MS);
};

/**
 * Options for event batch processing.
 * @property delay - Delay in ms to wait before processing events in the batch.
 * @property source - Source of the events for metrics tracking (e.g., "otel", "api").
 * @property isLangfuseInternal - Whether the events are being ingested by Langfuse internally (e.g. traces created for prompt experiments).
 * @property forwardToEventsTable - Whether to forward events to the staging events table for batch propagation. If undefined, falls back to environment flags.
 */
type ProcessEventBatchOptions = {
  delay?: number | null;
  source?: "api" | "otel";
  isLangfuseInternal?: boolean;
  forwardToEventsTable?: boolean;
};

/**
 * Processes a batch of events.
 * @param input - Batch of IngestionEventType. Will validate the types first thing and return errors if they are invalid.
 * @param authCheck - AuthHeaderValidVerificationResultIngestion
 * @param options - (Optional) Options for the event batch processing.
 */
export const processEventBatch = async (
  input: unknown[],
  authCheck: AuthHeaderValidVerificationResultIngestion,
  options: ProcessEventBatchOptions = {},
): Promise<{
  successes: { id: string; status: number }[];
  errors: {
    id: string;
    status: number;
    message?: string;
    error?: string;
  }[];
}> => {
  if (input.length === 0) {
    return { successes: [], errors: [] };
  }
  const {
    delay = null,
    source = "api",
    isLangfuseInternal = false,
    forwardToEventsTable,
  } = options;

  // add context of api call to the span
  const currentSpan = getCurrentSpan();
  recordIncrement("langfuse.ingestion.event", input.length, { source });
  recordDistribution("langfuse.ingestion.event_distribution", input.length, {
    source,
  });

  currentSpan?.setAttribute("langfuse.ingestion.batch_size", input.length);
  currentSpan?.setAttribute(
    "langfuse.project.id",
    authCheck.scope.projectId ?? "",
  );
  if (authCheck.scope.orgId)
    currentSpan?.setAttribute("langfuse.org.id", authCheck.scope.orgId);
  if (authCheck.scope.plan)
    currentSpan?.setAttribute("langfuse.org.plan", authCheck.scope.plan);

  /**************
   * VALIDATION *
   **************/
  if (!authCheck.scope.projectId) {
    throw new UnauthorizedError("Missing project ID");
  }

  const validationErrors: { id: string; error: unknown }[] = [];
  const authenticationErrors: { id: string; error: unknown }[] = [];

  const ingestionSchema = createIngestionEventSchema(isLangfuseInternal);
  const batch: z.infer<typeof ingestionSchema>[] = input
    .flatMap((event) => {
      const parsed = ingestionSchema.safeParse(event);
      if (!parsed.success) {
        validationErrors.push({
          id:
            typeof event === "object" && event && "id" in event
              ? typeof event.id === "string"
                ? event.id
                : "unknown"
              : "unknown",
          error: new InvalidRequestError(parsed.error.message),
        });
        return [];
      }
      if (!isAuthorized(parsed.data, authCheck)) {
        authenticationErrors.push({
          id: parsed.data.id,
          error: new UnauthorizedError("Access Scope Denied"),
        });
        return [];
      }
      return [parsed.data];
    })
    .flatMap((event) => {
      if (event.type === eventTypes.SDK_LOG) {
        // Log SDK_LOG events, but remove them from further processing
        logger.info("SDK Log Event", { event });
        return [];
      }
      return [event];
    });

  const sortedBatch = sortBatch(batch);

  // 按 eventBodyId 分组（单个实体的多个更新合并为一个队列作业）
  const sortedBatchByEventBodyId = sortedBatch.reduce(
    (
      acc: Record<
        string,
        {
          data: IngestionEventType[];
          eventBodyId: string;
          type: (typeof eventTypes)[keyof typeof eventTypes];
          entityType: IngestionEntityTypes;
        }
      >,
      event,
    ) => {
      if (!event.body?.id) return acc;
      const entityType = getClickhouseEntityType(event.type);
      const dedupKey = `${entityType}-${event.body.id}`;
      if (!acc[dedupKey]) {
        acc[dedupKey] = {
          data: [],
          type: event.type,
          eventBodyId: event.body.id,
          entityType,
        };
      }
      acc[dedupKey].data.push(event);
      return acc;
    },
    {} as Record<string, {
      data: IngestionEventType[];
      eventBodyId: string;
      type: (typeof eventTypes)[keyof typeof eventTypes];
      entityType: IngestionEntityTypes;
    }>,
  );

  /********************
   * PG QUEUE ENQUEUE *
   ********************/
  // 事件数据直接放入 pg_jobs.payload，不经过 S3 中转
  const queue = getPgQueue("ingestion-queue");

  await Promise.all(
    Object.keys(sortedBatchByEventBodyId).map(async (id) => {
      const eventData = sortedBatchByEventBodyId[id];

      const { isSampled, isSamplingConfigured } = isTraceIdInSample({
        projectId: authCheck.scope.projectId,
        event: eventData.data[0],
      });

      if (!isSampled) {
        recordIncrement("langfuse.ingestion.sampling", eventData.data.length, {
          projectId: authCheck.scope.projectId ?? "<not set>",
          sampling_decision: "out",
        });
        return;
      }

      if (isSamplingConfigured) {
        recordIncrement("langfuse.ingestion.sampling", eventData.data.length, {
          projectId: authCheck.scope.projectId ?? "<not set>",
          sampling_decision: "in",
        });
      }

      return queue.add(
        QueueJobs.IngestionJob,
        {
          id: randomUUID(),
          timestamp: new Date(),
          name: QueueJobs.IngestionJob as const,
          payload: {
            data: {
              type: eventData.type,
              eventBodyId: eventData.eventBodyId,
              events: eventData.data,  // ← 事件数组内嵌在 payload 中
              forwardToEventsTable,
            },
            authCheck: authCheck as {
              validKey: true;
              scope: {
                projectId: string;
                accessLevel: "project" | "scores";
              };
            },
          },
        },
        {
          delay: getDelay(delay, source),
          attempts: 6,
          backoff: { type: "exponential", delay: 5000 },
        },
      );
    }),
  );

  return aggregateBatchResult(
    [...validationErrors, ...authenticationErrors],
    sortedBatch.map((event) => ({ id: event.id, result: event })),
    authCheck.scope.projectId,
  );
};

const isAuthorized = (
  event: IngestionEventType,
  authScope: AuthHeaderValidVerificationResultIngestion,
): boolean => {
  if (event.type === eventTypes.SDK_LOG) {
    return true;
  }

  if (event.type === eventTypes.SCORE_CREATE) {
    return (
      authScope.scope.accessLevel === "scores" ||
      authScope.scope.accessLevel === "project"
    );
  }

  return authScope.scope.accessLevel === "project";
};

/**
 * Sorts a batch of ingestion events. Orders by: updating events last, sorted by timestamp asc.
 */
const sortBatch = (batch: IngestionEventType[]) => {
  const updateEvents: (typeof eventTypes)[keyof typeof eventTypes][] = [
    eventTypes.GENERATION_UPDATE,
    eventTypes.SPAN_UPDATE,
    eventTypes.OBSERVATION_UPDATE, // legacy event type
  ];
  const updates = batch
    .filter((event) => updateEvents.includes(event.type))
    .sort((a, b) => {
      return new Date(a.timestamp).getTime() - new Date(b.timestamp).getTime();
    });
  const others = batch
    .filter((event) => !updateEvents.includes(event.type))
    .sort((a, b) => {
      return new Date(a.timestamp).getTime() - new Date(b.timestamp).getTime();
    });

  // Return the array with non-update events first, followed by update events
  return [...others, ...updates];
};

export const aggregateBatchResult = (
  errors: Array<{ id: string; error: unknown }>,
  results: Array<{ id: string; result: unknown }>,
  projectId?: string,
) => {
  const returnedErrors: {
    id: string;
    status: number;
    message?: string;
    error?: string;
  }[] = [];

  const successes: {
    id: string;
    status: number;
  }[] = [];

  errors.forEach((error) => {
    if (error.error instanceof InvalidRequestError) {
      returnedErrors.push({
        id: error.id,
        status: 400,
        message: "Invalid request data",
        error: error.error.message,
      });
    } else if (error.error instanceof UnauthorizedError) {
      returnedErrors.push({
        id: error.id,
        status: 401,
        message: "Authentication error",
        error: error.error.message,
      });
    } else if (error.error instanceof LangfuseNotFoundError) {
      returnedErrors.push({
        id: error.id,
        status: 404,
        message: "Resource not found",
        error: error.error.message,
      });
    } else {
      returnedErrors.push({
        id: error.id,
        status: 500,
        error: "Internal Server Error",
      });
    }
  });

  if (returnedErrors.length > 0) {
    logger.warn("Error processing events", {
      errors: returnedErrors,
      "langfuse.project.id": projectId,
    });
  }

  results.forEach((result) => {
    successes.push({
      id: result.id,
      status: 201,
    });
  });

  return { successes, errors: returnedErrors };
};
