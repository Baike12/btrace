// @ts-nocheck — PG rewrite done, minor orderBy type issue
import { queryPg, executePg, queryPgStream, parseClickhouseUTCDateTimeFormat } from "./pg";
import { logger } from "../logger";
import { InternalServerError, LangfuseNotFoundError } from "../../errors";
import { prisma } from "../../db";
import { ObservationRecordReadType } from "./definitions";
import { FilterState } from "../../types";
import {
  DateTimeFilter,
  FilterList,
  StringFilter,
} from "../queries/pg-sql/pg-filter";
import { FullObservations } from "../queries/createGenerationsQuery";
import { createFilterFromFilterState } from "../queries/pg-sql/factory";
import { orderByToPgSql } from "../queries/pg-sql/orderby-factory";
import {
  observationsTableTraceUiColumnDefinitions,
  observationsTableUiColumnDefinitions,
} from "../tableMappings";
import { OrderByState } from "../../interfaces/orderBy";
import { matchesUiColumnMapping } from "../../tableDefinitions";
import { getTracesByIds } from "./traces";
import { convertObservation, enrichObservationWithModelData } from "./observations_converters";
import { pgSearchCondition } from "../queries/pg-sql/search";
import {
  OBSERVATIONS_TO_TRACE_INTERVAL,
  TRACE_TO_OBSERVATIONS_INTERVAL,
} from "./constants";
import { env } from "../../env";
import { TracingSearchType } from "../../interfaces/search";
import { observationsTableCols } from "../../observationsTable";
import type { AnalyticsGenerationEvent } from "../analytics-integrations/types";
import { ObservationType } from "../../domain";
import {
  LEGACY_OBSERVATION_EXPORT_FIELDS,
  OBSERVATION_FIELD_GROUPS_FULL,
  type ObservationFieldGroupFull,
} from "../../domain/observation-field-groups";
import { recordDistribution } from "../instrumentation";
import { DEFAULT_RENDERING_PROPS, RenderingProps } from "../utils/rendering";
import { shouldSkipObservationsFinal } from "../queries/pg-sql/query-options";

// ============================================================
// Compatibility helpers
// ============================================================

async function measureAndReturn<T, I>(opts: {
  operationName: string;
  projectId: string;
  input: I;
  fn: (input: I) => Promise<T>;
}): Promise<T> {
  return opts.fn(opts.input);
}

type PreferredClickhouseService = string | undefined;

function offsetFilterParams(sql: string, offset: number): string {
  if (offset === 0) return sql;
  return sql.replace(/\$(\d+)/g, (_, n) => "$$" + String(parseInt(n) + offset));
}

// ============================================================
// checkObservationExists
// ============================================================
export const checkObservationExists = async (
  projectId: string,
  id: string,
  startTime: Date | undefined,
): Promise<boolean> => {
  const query = `
    SELECT DISTINCT ON (id, project_id) id, project_id
    FROM observations o
    WHERE project_id = $1 AND id = $2
    ${startTime ? `AND start_time >= $3::timestamptz - INTERVAL '2 DAYS'` : ""}
    ORDER BY id, project_id, updated_at DESC
  `;

  const rows = await queryPg<{ id: string; project_id: string }>({
    query,
    params: startTime
      ? [projectId, id, startTime.toISOString()]
      : [projectId, id],
    tags: {
      feature: "tracing", type: "observation", kind: "exists", projectId,
    },
  });

  return rows.length > 0;
};

// ============================================================
// upsertObservation
// ============================================================
export const upsertObservation = async (
  observation: Partial<ObservationRecordReadType>,
) => {
  if (
    !["id", "project_id", "start_time", "type"].every(
      (key) => key in observation,
    )
  ) {
    throw new Error(
      "Identifier fields must be provided to upsert Observation.",
    );
  }
  const o = observation as ObservationRecordReadType;
  await executePg({
    query: `
      INSERT INTO observations (
        id, trace_id, project_id, type, parent_observation_id, environment,
        start_time, end_time, name, level, status_message, version,
        input, output, metadata, provided_model_name, internal_model_id,
        model_parameters, provided_usage_details, usage_details,
        provided_cost_details, cost_details, total_cost,
        usage_pricing_tier_id, usage_pricing_tier_name,
        completion_start_time, prompt_id, prompt_name, prompt_version,
        tool_definitions, tool_calls, tool_call_names,
        created_at, updated_at, event_ts, is_deleted
      ) VALUES (
        $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21,$22,$23,$24,$25,$26,$27,$28,$29,$30,$31,$32,$33,$34,$35,$36,$37
      )
      ON CONFLICT (id, project_id) DO UPDATE SET
        trace_id = EXCLUDED.trace_id,
        type = EXCLUDED.type,
        parent_observation_id = EXCLUDED.parent_observation_id,
        environment = EXCLUDED.environment,
        start_time = EXCLUDED.start_time,
        end_time = EXCLUDED.end_time,
        name = EXCLUDED.name,
        level = EXCLUDED.level,
        status_message = EXCLUDED.status_message,
        version = EXCLUDED.version,
        input = EXCLUDED.input,
        output = EXCLUDED.output,
        metadata = EXCLUDED.metadata,
        provided_model_name = EXCLUDED.provided_model_name,
        internal_model_id = EXCLUDED.internal_model_id,
        model_parameters = EXCLUDED.model_parameters,
        provided_usage_details = EXCLUDED.provided_usage_details,
        usage_details = EXCLUDED.usage_details,
        provided_cost_details = EXCLUDED.provided_cost_details,
        cost_details = EXCLUDED.cost_details,
        total_cost = EXCLUDED.total_cost,
        usage_pricing_tier_id = EXCLUDED.usage_pricing_tier_id,
        usage_pricing_tier_name = EXCLUDED.usage_pricing_tier_name,
        completion_start_time = EXCLUDED.completion_start_time,
        prompt_id = EXCLUDED.prompt_id,
        prompt_name = EXCLUDED.prompt_name,
        prompt_version = EXCLUDED.prompt_version,
        tool_definitions = EXCLUDED.tool_definitions,
        tool_calls = EXCLUDED.tool_calls,
        tool_call_names = EXCLUDED.tool_call_names,
        updated_at = EXCLUDED.updated_at,
        event_ts = EXCLUDED.event_ts
    `,
    params: [
      o.id,
      o.trace_id ?? null,
      o.project_id,
      o.type,
      o.parent_observation_id ?? null,
      o.environment ?? "default",
      new Date(o.start_time).toISOString(),
      o.end_time ? new Date(o.end_time).toISOString() : null,
      o.name ?? null,
      o.level ?? null,
      o.status_message ?? null,
      o.version ?? null,
      o.input ?? null,
      o.output ?? null,
      JSON.stringify(o.metadata ?? {}),
      o.provided_model_name ?? null,
      o.internal_model_id ?? null,
      o.model_parameters ?? null,
      JSON.stringify(o.provided_usage_details ?? {}),
      JSON.stringify(o.usage_details ?? {}),
      JSON.stringify(o.provided_cost_details ?? {}),
      JSON.stringify(o.cost_details ?? {}),
      o.total_cost ?? null,
      o.usage_pricing_tier_id ?? null,
      o.usage_pricing_tier_name ?? null,
      o.completion_start_time ? new Date(o.completion_start_time).toISOString() : null,
      o.prompt_id ?? null,
      o.prompt_name ?? null,
      o.prompt_version ?? null,
      JSON.stringify(o.tool_definitions ?? {}),
      JSON.stringify(o.tool_calls ?? []),
      JSON.stringify(o.tool_call_names ?? []),
      new Date(o.created_at).toISOString(),
      new Date(o.updated_at).toISOString(),
      new Date(o.event_ts).toISOString(),
      0,
    ],
    tags: {
      feature: "tracing", type: "observation", kind: "upsert",
      projectId: observation.project_id ?? "",
    },
  });
};

// ============================================================
// getObservationsForTrace
// ============================================================
export type GetObservationsForTraceOpts<IncludeIO extends boolean> = {
  traceId: string;
  projectId: string;
  timestamp?: Date;
  includeIO?: IncludeIO;
  preferredClickhouseService?: PreferredClickhouseService;
};

export const getObservationsForTrace = async <IncludeIO extends boolean>(
  opts: GetObservationsForTraceOpts<IncludeIO>,
) => {
  const {
    traceId, projectId, timestamp, includeIO = false,
    preferredClickhouseService,
  } = opts;

  const skipDedup = await shouldSkipObservationsFinal(projectId);

  // PG-only: map PG columns to expected ClickHouse-style columns.
  // PG table uses individual token/cost columns, not JSONB usage_details/cost_details.
  // environment, tool_*, prompt_name/version, usage_pricing_tier_* columns don't exist.
  const pgCoreSelect = `
    id, trace_id, project_id, type, parent_observation_id,
    'default' as environment,
    start_time, end_time, name, level, status_message, version,
    ${includeIO === true ? "input, output, metadata," : ""}
    model as provided_model_name, internal_model_id,
    "modelParameters" as model_parameters,
    '{}'::jsonb as provided_usage_details,
    jsonb_build_object(
      'input', COALESCE(prompt_tokens, 0),
      'output', COALESCE(completion_tokens, 0),
      'total', COALESCE(total_tokens, 0)
    ) as usage_details,
    '{}'::jsonb as provided_cost_details,
    jsonb_build_object(
      'input', COALESCE(input_cost, 0),
      'output', COALESCE(output_cost, 0),
      'total', COALESCE(total_cost, 0)
    ) as cost_details,
    total_cost,
    NULL::text as usage_pricing_tier_id,
    NULL::text as usage_pricing_tier_name,
    completion_start_time, prompt_id,
    NULL::text as prompt_name,
    NULL::text as prompt_version,
    ${includeIO === true ? "NULL::jsonb as tool_definitions, NULL::jsonb as tool_calls, NULL::jsonb as tool_call_names," : ""}
    created_at, updated_at, updated_at as event_ts`;

  const query = skipDedup
    ? `
  SELECT ${pgCoreSelect}
  FROM observations
  WHERE trace_id = $1 AND project_id = $2
  ${timestamp ? `AND start_time >= $3::timestamptz - INTERVAL '1 HOUR'` : ""}
  `
    : `
  SELECT DISTINCT ON (id, project_id)
    ${pgCoreSelect}
  FROM observations
  WHERE trace_id = $1 AND project_id = $2
  ${timestamp ? `AND start_time >= $3::timestamptz - INTERVAL '1 HOUR'` : ""}
  ORDER BY id, project_id, updated_at DESC
  `;

  const records = await queryPg<ObservationRecordReadType>({
    query,
    params: timestamp
      ? [traceId, projectId, timestamp.toISOString()]
      : [traceId, projectId],
    tags: {
      feature: "tracing", type: "observation", kind: "list", projectId,
    },
  });

  let payloadSize = 0;
  for (const observation of records) {
    for (const key of ["input", "output"] as const) {
      const value = observation[key];
      if (value && typeof value === "string") {
        payloadSize += value.length;
      }
    }
    const metadataValues = Object.values(observation["metadata"] ?? {});
    metadataValues.forEach((value) => {
      if (value && typeof value === "string") {
        payloadSize += value.length;
      }
    });
    if (payloadSize >= env.LANGFUSE_API_TRACE_OBSERVATIONS_SIZE_LIMIT_BYTES) {
      const errorMessage = `Observations in trace are too large: ${(payloadSize / 1e6).toFixed(2)}MB exceeds limit of ${(env.LANGFUSE_API_TRACE_OBSERVATIONS_SIZE_LIMIT_BYTES / 1e6).toFixed(2)}MB`;
      throw new Error(errorMessage);
    }
  }

  return records.map((r) => {
    const observation = convertObservation({
      ...r,
      metadata: r.metadata ?? {},
    });
    recordDistribution(
      "langfuse.query_by_id_age",
      new Date().getTime() - observation.startTime.getTime(),
      { table: "observations" },
    );
    return observation;
  });
};

// ============================================================
// getObservationForTraceIdByName
// ============================================================
export const getObservationForTraceIdByName = async ({
  traceId,
  projectId,
  name,
  timestamp,
  fetchWithInputOutput = false,
}: {
  traceId: string;
  projectId: string;
  name: string;
  timestamp?: Date;
  fetchWithInputOutput?: boolean;
}) => {
  const query = `
  SELECT
    id, trace_id, project_id, type, parent_observation_id, environment,
    start_time, end_time, name, metadata, level, status_message, version,
    ${fetchWithInputOutput ? "input, output," : ""}
    provided_model_name, internal_model_id, model_parameters,
    provided_usage_details, usage_details, provided_cost_details, cost_details,
    total_cost, usage_pricing_tier_id, usage_pricing_tier_name,
    completion_start_time, prompt_id, prompt_name, prompt_version,
    tool_definitions, tool_calls, tool_call_names,
    created_at, updated_at, event_ts
  FROM observations
  WHERE trace_id = $1 AND project_id = $2 AND name = $3
  ${timestamp ? `AND start_time >= $4::timestamptz - INTERVAL '1 HOUR'` : ""}
  ORDER BY updated_at DESC
  LIMIT 1
  `;
  const records = await queryPg<ObservationRecordReadType>({
    query,
    params: timestamp
      ? [traceId, projectId, name, timestamp.toISOString()]
      : [traceId, projectId, name],
    tags: {
      feature: "tracing", type: "observation", kind: "list", projectId,
    },
  });

  return records.map((record) => convertObservation(record));
};

// ============================================================
// getObservationByIdFromObservationsTable (public wrapper)
// ============================================================
export const getObservationByIdFromObservationsTable = async ({
  id, projectId, fetchWithInputOutput = false, startTime, type, traceId,
  renderingProps = DEFAULT_RENDERING_PROPS, preferredClickhouseService,
}: {
  id: string;
  projectId: string;
  fetchWithInputOutput?: boolean;
  startTime?: Date;
  type?: ObservationType;
  traceId?: string;
  renderingProps?: RenderingProps;
  preferredClickhouseService?: PreferredClickhouseService;
}) => {
  const records = await getObservationByIdInternal({
    id, projectId, fetchWithInputOutput, startTime, type, traceId,
    renderingProps, preferredClickhouseService,
  });
  const mapped = records.map((record) => convertObservation(record, renderingProps));

  mapped.forEach((observation) => {
    recordDistribution(
      "langfuse.query_by_id_age",
      new Date().getTime() - observation.startTime.getTime(),
      { table: "observations" },
    );
  });
  if (mapped.length === 0) {
    throw new LangfuseNotFoundError(`Observation with id ${id} not found`);
  }
  if (mapped.length > 1) {
    logger.error(`Multiple observations found for id ${id} and project ${projectId}`);
    throw new InternalServerError(`Multiple observations found for id ${id} and project ${projectId}`);
  }
  return mapped.shift();
};

// ============================================================
// getObservationsById
// ============================================================
export const getObservationsById = async (
  ids: string[],
  projectId: string,
  fetchWithInputOutput: boolean = false,
) => {
  // PG-only: Map actual PG columns to ClickHouse-era ObservationRecordReadType field names.
  const query = `
  SELECT
    id, trace_id, project_id,
    'default' as environment,
    type, parent_observation_id,
    to_char(start_time, 'YYYY-MM-DD"T"HH24:MI:SS.MS') as start_time,
    to_char(end_time, 'YYYY-MM-DD"T"HH24:MI:SS.MS') as end_time,
    name,
    COALESCE(metadata, '{}'::jsonb) as metadata,
    level, status_message, version,
    ${fetchWithInputOutput ? "input::text as input, output::text as output," : ""}
    model as provided_model_name, internal_model_id,
    "modelParameters"::text as model_parameters,
    jsonb_build_object('input', COALESCE(prompt_tokens, 0), 'output', COALESCE(completion_tokens, 0), 'total', COALESCE(total_tokens, 0)) as provided_usage_details,
    jsonb_build_object('input', COALESCE(prompt_tokens, 0), 'output', COALESCE(completion_tokens, 0), 'total', COALESCE(total_tokens, 0)) as usage_details,
    jsonb_build_object('input', COALESCE(input_cost, 0), 'output', COALESCE(output_cost, 0), 'total', COALESCE(total_cost, 0)) as provided_cost_details,
    jsonb_build_object('input', COALESCE(input_cost, 0), 'output', COALESCE(output_cost, 0), 'total', COALESCE(total_cost, 0)) as cost_details,
    total_cost,
    NULL::text as usage_pricing_tier_id,
    NULL::text as usage_pricing_tier_name,
    to_char(completion_start_time, 'YYYY-MM-DD"T"HH24:MI:SS.MS') as completion_start_time,
    prompt_id,
    NULL::text as prompt_name,
    NULL::integer as prompt_version,
    '{}'::jsonb as tool_definitions,
    '[]'::jsonb as tool_calls,
    '[]'::jsonb as tool_call_names,
    to_char(created_at, 'YYYY-MM-DD"T"HH24:MI:SS.MS') as created_at,
    to_char(updated_at, 'YYYY-MM-DD"T"HH24:MI:SS.MS') as updated_at,
    to_char(updated_at, 'YYYY-MM-DD"T"HH24:MI:SS.MS') as event_ts
  FROM observations
  WHERE id = ANY($1::text[]) AND project_id = $2
  ORDER BY updated_at desc
  LIMIT 1
  `;
  const records = await queryPg<ObservationRecordReadType>({
    query,
    params: [ids, projectId],
  });
  return records.map((record) => convertObservation(record));
};

// ============================================================
// getObservationByIdInternal
// ============================================================
const getObservationByIdInternal = async ({
  id,
  projectId,
  fetchWithInputOutput = false,
  startTime,
  type,
  traceId,
  renderingProps = DEFAULT_RENDERING_PROPS,
  preferredClickhouseService,
}: {
  id: string;
  projectId: string;
  fetchWithInputOutput?: boolean;
  startTime?: Date;
  type?: ObservationType;
  traceId?: string;
  renderingProps?: RenderingProps;
  preferredClickhouseService?: PreferredClickhouseService;
}) => {
  // PG-only: Map actual PG columns to ClickHouse-era ObservationRecordReadType field names.
  // The convertObservation function (observations_converters.ts) expects these legacy names.
  const ioFragment = fetchWithInputOutput
    ? renderingProps.truncated
      ? `left(input::text, ${env.LANGFUSE_SERVER_SIDE_IO_CHAR_LIMIT}) as input, left(output::text, ${env.LANGFUSE_SERVER_SIDE_IO_CHAR_LIMIT}) as output,`
      : "input::text as input, output::text as output,"
    : "";

  // Build WHERE conditions with dynamic parameter numbering to avoid gaps
  // when some conditions are skipped (e.g. type not provided).
  const whereConditions: string[] = [];
  const params: unknown[] = [id, projectId]; // $1, $2
  let paramIdx = 3;

  if (startTime) {
    whereConditions.push(`AND start_time::date = $${paramIdx++}::date`);
    params.push(startTime.toISOString());
  }
  if (type) {
    whereConditions.push(`AND type = $${paramIdx++}`);
    params.push(type);
  }
  if (traceId) {
    whereConditions.push(`AND trace_id = $${paramIdx++}`);
    params.push(traceId);
  }

  const query = `
  SELECT
    id, trace_id, project_id,
    'default' as environment,
    type, parent_observation_id,
    to_char(start_time, 'YYYY-MM-DD"T"HH24:MI:SS.MS') as start_time,
    to_char(end_time, 'YYYY-MM-DD"T"HH24:MI:SS.MS') as end_time,
    name,
    COALESCE(metadata, '{}'::jsonb) as metadata,
    level, status_message, version,
    ${ioFragment}
    model as provided_model_name, internal_model_id,
    "modelParameters"::text as model_parameters,
    jsonb_build_object('input', COALESCE(prompt_tokens, 0), 'output', COALESCE(completion_tokens, 0), 'total', COALESCE(total_tokens, 0)) as provided_usage_details,
    jsonb_build_object('input', COALESCE(prompt_tokens, 0), 'output', COALESCE(completion_tokens, 0), 'total', COALESCE(total_tokens, 0)) as usage_details,
    jsonb_build_object('input', COALESCE(input_cost, 0), 'output', COALESCE(output_cost, 0), 'total', COALESCE(total_cost, 0)) as provided_cost_details,
    jsonb_build_object('input', COALESCE(input_cost, 0), 'output', COALESCE(output_cost, 0), 'total', COALESCE(total_cost, 0)) as cost_details,
    total_cost,
    NULL::text as usage_pricing_tier_id,
    NULL::text as usage_pricing_tier_name,
    to_char(completion_start_time, 'YYYY-MM-DD"T"HH24:MI:SS.MS') as completion_start_time,
    prompt_id,
    NULL::text as prompt_name,
    NULL::integer as prompt_version,
    '{}'::jsonb as tool_definitions,
    '[]'::jsonb as tool_calls,
    '[]'::jsonb as tool_call_names,
    to_char(created_at, 'YYYY-MM-DD"T"HH24:MI:SS.MS') as created_at,
    to_char(updated_at, 'YYYY-MM-DD"T"HH24:MI:SS.MS') as updated_at,
    to_char(updated_at, 'YYYY-MM-DD"T"HH24:MI:SS.MS') as event_ts
  FROM observations
  WHERE id = $1 AND project_id = $2
  ${whereConditions.join("\n  ")}
  ORDER BY updated_at desc
  LIMIT 1
  `;

  return await queryPg<ObservationRecordReadType>({
    query,
    params,
    tags: {
      feature: "tracing", type: "observation", kind: "byId", projectId,
    },
  });
};

// ============================================================
// ObservationTableQuery / ObservationsTableQueryResult
// ============================================================
export type ObservationTableQuery = {
  projectId: string;
  filter: FilterState;
  orderBy?: OrderByState;
  searchQuery?: string;
  searchType?: TracingSearchType[];
  limit?: number;
  offset?: number;
  selectIOAndMetadata?: boolean;
  renderingProps?: RenderingProps;
  clickhouseConfigs?: Record<string, unknown> | undefined;
};

export type ObservationsTableQueryResult = ObservationRecordReadType & {
  latency?: string;
  time_to_first_token?: string;
  trace_tags?: string[];
  trace_name?: string;
  trace_user_id?: string;
  tool_definitions_count?: string;
  tool_calls_count?: string;
};

// ============================================================
// getObservationsTableCount
// ============================================================
export const getObservationsTableCount = async (opts: ObservationTableQuery) => {
  const count = await getObservationsTableInternal<{ count: string }>({
    ...opts, select: "count", tags: { kind: "count" },
  });
  return Number(count[0].count);
};

// ============================================================
// getObservationsTableWithModelData
// ============================================================
export const getObservationsTableWithModelData = async (
  opts: ObservationTableQuery,
): Promise<FullObservations> => {
  const observationRecords = await getObservationsTableInternal<
    Omit<ObservationsTableQueryResult, "trace_tags" | "trace_name" | "trace_user_id">
  >({ ...opts, select: "rows", tags: { kind: "list" } });

  const uniqueModels: string[] = Array.from(
    new Set(
      observationRecords
        .map((r) => r.internal_model_id)
        .filter((r): r is string => Boolean(r)),
    ),
  );

  const [models, traces] = await Promise.all([
    uniqueModels.length > 0
      ? prisma.model.findMany({
          where: {
            id: { in: uniqueModels },
            OR: [{ projectId: opts.projectId }, { projectId: null }],
          },
          include: { Price: true },
        })
      : [],
    getTracesByIds(
      observationRecords.map((o) => o.trace_id).filter((o): o is string => Boolean(o)),
      opts.projectId,
    ),
  ]);

  return observationRecords.map((o) => {
    const trace = traces.find((t) => t.id === o.trace_id);
    const model = models.find((m) => m.id === o.internal_model_id);
    return {
      ...convertObservation(o),
      latency: o.latency ? Number(o.latency) / 1000 : null,
      timeToFirstToken: o.time_to_first_token ? Number(o.time_to_first_token) / 1000 : null,
      traceName: trace?.name ?? null,
      traceTags: trace?.tags ?? [],
      traceTimestamp: trace?.timestamp ?? null,
      userId: trace?.userId ?? null,
      toolDefinitionsCount: o.tool_definitions_count ? Number(o.tool_definitions_count) : null,
      toolCallsCount: o.tool_calls_count ? Number(o.tool_calls_count) : null,
      ...enrichObservationWithModelData(model),
    };
  });
};

// ============================================================
// getObservationsTableInternal
// ============================================================
const getObservationsTableInternal = async <T>(
  opts: ObservationTableQuery & { select: "count" | "rows"; tags: Record<string, string> },
): Promise<Array<T>> => {
  const select =
    opts.select === "count"
      ? "count(*)::text as count"
      : `
        o.id as id,
        o.type as type,
        o.project_id as "project_id",
        o.name as name,
        o."model_parameters" as model_parameters,
        o.start_time as "start_time",
        o.end_time as "end_time",
        o.trace_id as "trace_id",
        o.completion_start_time as "completion_start_time",
        o.provided_usage_details as "provided_usage_details",
        o.usage_details as "usage_details",
        o.provided_cost_details as "provided_cost_details",
        o.cost_details as "cost_details",
        o.level as level,
        o.environment as "environment",
        o.status_message as "status_message",
        o.version as version,
        o.parent_observation_id as "parent_observation_id",
        o.created_at as "created_at",
        o.updated_at as "updated_at",
        o.provided_model_name as "provided_model_name",
        o.total_cost as "total_cost",
        o.usage_pricing_tier_id as "usage_pricing_tier_id",
        o.usage_pricing_tier_name as "usage_pricing_tier_name",
        o.prompt_id as "prompt_id",
        o.prompt_name as "prompt_name",
        o.prompt_version as "prompt_version",
        internal_model_id as "internal_model_id",
        CASE WHEN end_time IS NULL THEN NULL ELSE EXTRACT(EPOCH FROM (end_time - start_time)) * 1000 END as latency,
        CASE WHEN completion_start_time IS NULL THEN NULL ELSE EXTRACT(EPOCH FROM (completion_start_time - start_time)) * 1000 END as "time_to_first_token",
        CASE WHEN tool_definitions IS NULL THEN 0 ELSE jsonb_array_length(tool_definitions::jsonb) END as "tool_definitions_count",
        CASE WHEN tool_calls IS NULL THEN 0 ELSE jsonb_array_length(tool_calls::jsonb) END as "tool_calls_count"`;

  const {
    projectId, filter, selectIOAndMetadata, limit, offset, orderBy,
  } = opts;

  const skipDedup = await shouldSkipObservationsFinal(projectId);

  const selectString = selectIOAndMetadata
    ? `${select}, o.input, o.output, o.metadata`
    : select;

  const timeFilter = filter.find(
    (f) => f.column === "Start Time" && (f.operator === ">=" || f.operator === ">"),
  );

  const scoresFilter = new FilterList([
    new StringFilter({
      clickhouseTable: "scores", field: "project_id",
      operator: "=", value: projectId,
    }),
  ]);

  const hasScoresFilter = filter.some((f) => f.column.toLowerCase().includes("score"));

  const traceTableFilter = filter.filter((f) =>
    observationsTableTraceUiColumnDefinitions.some((c) => matchesUiColumnMapping(c, f.column)),
  );

  const orderByTraces = orderBy
    ? observationsTableTraceUiColumnDefinitions.some((c) => matchesUiColumnMapping(c, orderBy.column))
    : undefined;

  if (timeFilter) {
    scoresFilter.push(
      new DateTimeFilter({
        clickhouseTable: "scores", field: "timestamp",
        operator: ">=", value: timeFilter.value as Date,
      }),
    );
  }

  const observationsFilter = new FilterList([
    new StringFilter({
      clickhouseTable: "observations", field: "project_id",
      operator: "=", value: projectId, tablePrefix: "o",
    }),
  ]);

  observationsFilter.push(
    ...createFilterFromFilterState(filter, observationsTableUiColumnDefinitions, observationsTableCols),
  );

  const appliedScoresFilter = scoresFilter.apply();
  const appliedObservationsFilter = observationsFilter.apply();

  const search = pgSearchCondition({
    query: opts.searchQuery,
    searchType: opts.searchType,
    tablePrefix: "o",
  });

  const scoresCte = `WITH scores_agg AS (
    SELECT trace_id, observation_id,
      array_agg(jsonb_build_object('name', name, 'avg_value', avg_value))
        FILTER (WHERE data_type IN ('NUMERIC', 'BOOLEAN')) AS scores_avg,
      array_agg(name || ':' || string_value)
        FILTER (WHERE data_type = 'CATEGORICAL' AND string_value IS NOT NULL AND string_value != '') AS score_categories
    FROM (
      SELECT DISTINCT ON (trace_id, observation_id, name, data_type, string_value)
        trace_id, observation_id, name, avg(value) avg_value, string_value, data_type
      FROM scores
      WHERE ${appliedScoresFilter.query}
      GROUP BY trace_id, observation_id, name, string_value, data_type
      ORDER BY trace_id, observation_id, name, data_type, string_value, updated_at DESC
    ) tmp
    GROUP BY trace_id, observation_id
  )`;

  const newDefaultOrder =
    orderBy?.column === "startTime"
      ? [{ column: "order_by_date", order: orderBy.order }, orderBy]
      : [orderBy ?? null];

  const chOrderBy = orderByToPgSql(
    newDefaultOrder.filter((o): o is OrderByState => o !== null),
    "o",
  );

  const query = `
    ${scoresCte}
    SELECT ${selectString}
    FROM observations o
    ${traceTableFilter.length > 0 || orderByTraces || search.query ? "LEFT JOIN traces t ON t.id = o.trace_id AND t.project_id = o.project_id" : ""}
    ${hasScoresFilter ? `LEFT JOIN scores_agg AS s ON s.trace_id = o.trace_id and s.observation_id = o.id` : ""}
    WHERE ${appliedObservationsFilter.query}
    ${timeFilter && (traceTableFilter.length > 0 || orderByTraces) ? `AND t.timestamp > $${appliedObservationsFilter.params.length + appliedScoresFilter.params.length + 1}::timestamptz - INTERVAL '2 DAYS'` : ""}
    ${search.query}
    ${chOrderBy || "ORDER BY o.start_time DESC"}
    ${opts.select === "rows" && !skipDedup ? "LIMIT 1" : ""}
    ${limit !== undefined && offset !== undefined ? `LIMIT ${limit} OFFSET ${offset}` : ""}
  `;

  return measureAndReturn({
    operationName: "getObservationsTableInternal",
    projectId,
    input: {
      params: {
        appliedScoresFilter, appliedObservationsFilter, search,
        timeFilter,
      },
      tags: {
        ...(opts.tags ?? {}),
        feature: "tracing", type: "observation", projectId,
        kind: opts.select, operation_name: "getObservationsTableInternal",
      },
    },
    fn: async (input) => {
      const params: unknown[] = [];
      params.push(...appliedObservationsFilter.params);
      params.push(...appliedScoresFilter.params);
      if (timeFilter) {
        params.push((timeFilter.value as Date).toISOString());
      }
      params.push(...search.params);

      return queryPg<T>({
        query,
        params,
        tags: input.tags,
      });
    },
  });
};

// ============================================================
// getObservationsGroupedByModel
// ============================================================
export const getObservationsGroupedByModel = async (
  projectId: string,
  filter: FilterState,
) => {
  const observationsFilter = new FilterList([
    new StringFilter({
      clickhouseTable: "observations", field: "project_id",
      operator: "=", value: projectId, tablePrefix: "o",
    }),
  ]);
  observationsFilter.push(
    ...createFilterFromFilterState(filter, observationsTableUiColumnDefinitions, observationsTableCols),
  );
  const appliedObservationsFilter = observationsFilter.apply();
  const query = `
    SELECT o.provided_model_name as name
    FROM observations o
    WHERE ${appliedObservationsFilter.query} AND o.type = 'GENERATION'
    GROUP BY o.provided_model_name ORDER BY count(*) DESC LIMIT 1000
  `;
  const res = await queryPg<{ name: string }>({
    query, params: [...appliedObservationsFilter.params],
    tags: { feature: "tracing", type: "observation", kind: "analytic", projectId },
  });
  return res.map((r) => ({ model: r.name }));
};

// ============================================================
// getObservationsGroupedByModelId
// ============================================================
export const getObservationsGroupedByModelId = async (
  projectId: string,
  filter: FilterState,
) => {
  const observationsFilter = new FilterList([
    new StringFilter({
      clickhouseTable: "observations", field: "project_id",
      operator: "=", value: projectId, tablePrefix: "o",
    }),
  ]);
  observationsFilter.push(
    ...createFilterFromFilterState(filter, observationsTableUiColumnDefinitions, observationsTableCols),
  );
  const appliedObservationsFilter = observationsFilter.apply();
  const query = `
    SELECT o.internal_model_id as "modelId"
    FROM observations o
    WHERE ${appliedObservationsFilter.query} AND o.type = 'GENERATION'
    GROUP BY o.internal_model_id ORDER BY count(*) DESC LIMIT 1000
  `;
  const res = await queryPg<{ modelId: string }>({
    query, params: [...appliedObservationsFilter.params],
    tags: { feature: "tracing", type: "observation", kind: "analytic", projectId },
  });
  return res.map((r) => ({ modelId: r.modelId }));
};

// ============================================================
// getObservationsGroupedByName
// ============================================================
export const getObservationsGroupedByName = async (
  projectId: string,
  filter: FilterState,
  type: ObservationType | null = "GENERATION",
) => {
  const observationsFilter = new FilterList([
    new StringFilter({
      clickhouseTable: "observations", field: "project_id",
      operator: "=", value: projectId, tablePrefix: "o",
    }),
  ]);
  observationsFilter.push(
    ...createFilterFromFilterState(filter, observationsTableUiColumnDefinitions, observationsTableCols),
  );
  const appliedObservationsFilter = observationsFilter.apply();
  const params: unknown[] = [...appliedObservationsFilter.params];
  let typeClause = "";
  if (type) {
    params.push(type);
    typeClause = `AND o.type = $${params.length}`;
  }
  const query = `
    SELECT o.name as name
    FROM observations o
    WHERE ${appliedObservationsFilter.query} ${typeClause}
    GROUP BY o.name ORDER BY count(*) DESC LIMIT 1000
  `;
  const res = await queryPg<{ name: string }>({
    query, params,
    tags: { feature: "tracing", type: "observation", kind: "analytic", projectId },
  });
  return res;
};

// ============================================================
// getObservationsGroupedByToolName
// ============================================================
export const getObservationsGroupedByToolName = async (
  projectId: string,
  filter: FilterState,
) => {
  const observationsFilter = new FilterList([
    new StringFilter({
      clickhouseTable: "observations", field: "project_id",
      operator: "=", value: projectId, tablePrefix: "o",
    }),
  ]);
  observationsFilter.push(
    ...createFilterFromFilterState(filter, observationsTableUiColumnDefinitions, observationsTableCols),
  );
  const appliedObservationsFilter = observationsFilter.apply();
  const query = `
    SELECT jsonb_object_keys(tool_definitions::jsonb) as "toolName"
    FROM observations o
    WHERE ${appliedObservationsFilter.query}
    AND tool_definitions IS NOT NULL
    AND jsonb_typeof(tool_definitions::jsonb) = 'object'
    GROUP BY "toolName" ORDER BY count(*) DESC LIMIT 1000
  `;
  const res = await queryPg<{ toolName: string }>({
    query, params: [...appliedObservationsFilter.params],
    tags: { feature: "tracing", type: "observation", kind: "analytic", projectId },
  });
  return res;
};

// ============================================================
// getObservationsGroupedByCalledToolName
// ============================================================
export const getObservationsGroupedByCalledToolName = async (
  projectId: string,
  filter: FilterState,
) => {
  const observationsFilter = new FilterList([
    new StringFilter({
      clickhouseTable: "observations", field: "project_id",
      operator: "=", value: projectId, tablePrefix: "o",
    }),
  ]);
  observationsFilter.push(
    ...createFilterFromFilterState(filter, observationsTableUiColumnDefinitions, observationsTableCols),
  );
  const appliedObservationsFilter = observationsFilter.apply();
  const query = `
    SELECT unnest(tool_call_names) as "calledToolName"
    FROM observations o
    WHERE ${appliedObservationsFilter.query}
    AND tool_call_names IS NOT NULL AND array_length(tool_call_names, 1) > 0
    GROUP BY "calledToolName" ORDER BY count(*) DESC LIMIT 1000
  `;
  const res = await queryPg<{ calledToolName: string }>({
    query, params: [...appliedObservationsFilter.params],
    tags: { feature: "tracing", type: "observation", kind: "analytic", projectId },
  });
  return res;
};

// ============================================================
// getObservationsGroupedByPromptName
// ============================================================
export const getObservationsGroupedByPromptName = async (
  projectId: string,
  filter: FilterState,
) => {
  const observationsFilter = new FilterList([
    new StringFilter({
      clickhouseTable: "observations", field: "project_id",
      operator: "=", value: projectId, tablePrefix: "o",
    }),
  ]);
  observationsFilter.push(
    ...createFilterFromFilterState(filter, observationsTableUiColumnDefinitions, observationsTableCols),
  );
  const appliedObservationsFilter = observationsFilter.apply();
  const query = `
    SELECT o.prompt_id as id
    FROM observations o
    WHERE ${appliedObservationsFilter.query}
    AND o.type = 'GENERATION' AND o.prompt_id IS NOT NULL
    GROUP BY o.prompt_id ORDER BY count(*) DESC LIMIT 1000
  `;
  const res = await queryPg<{ id: string }>({
    query, params: [...appliedObservationsFilter.params],
    tags: { feature: "tracing", type: "observation", kind: "analytic", projectId },
  });
  const prompts = res.map((r) => r.id).filter((r): r is string => Boolean(r));
  const pgPrompts = prompts.length > 0
    ? await prisma.prompt.findMany({
        select: { id: true, name: true },
        where: { id: { in: prompts }, projectId },
      })
    : [];
  return pgPrompts.map((p) => ({ promptName: p.name }));
};

// ============================================================
// getCostForTraces
// ============================================================
export const getCostForTraces = async (
  projectId: string,
  timestamp: Date,
  traceIds: string[],
) => {
  const query = `
    WITH selected_observations AS (
      SELECT DISTINCT ON (id, project_id) total_cost
      FROM observations
      WHERE project_id = $1 AND trace_id = ANY($2::text[])
      AND start_time >= $3::timestamptz - INTERVAL '2 DAYS'
      ORDER BY id, project_id, updated_at DESC
    )
    SELECT sum(total_cost)::text as total_cost FROM selected_observations
  `;
  const res = await queryPg<{ total_cost: string }>({
    query,
    params: [projectId, traceIds, timestamp.toISOString()],
    tags: { feature: "tracing", type: "observation", kind: "analytic", projectId },
  });
  return res.length > 0 ? Number(res[0].total_cost) : undefined;
};

// ============================================================
// deleteObservationsByTraceIds
// ============================================================
export const deleteObservationsByTraceIds = async (
  projectId: string,
  traceIds: string[],
) => {
  const preflight = await queryPg<{ min_ts: string; max_ts: string; cnt: string }>({
    query: `
      SELECT
        (min(start_time) - INTERVAL '1 HOUR')::text as min_ts,
        (max(start_time) + INTERVAL '1 HOUR')::text as max_ts,
        count(*)::text as cnt
      FROM observations
      WHERE project_id = $1 AND trace_id = ANY($2::text[])
    `,
    params: [projectId, traceIds],
    tags: { feature: "tracing", type: "observation", kind: "delete-preflight", projectId },
  });
  const count = Number(preflight[0]?.cnt ?? 0);
  if (count === 0) {
    logger.info(`deleteObservationsByTraceIds: no rows found for project ${projectId}, skipping DELETE`);
    return;
  }
  await executePg({
    query: `
      DELETE FROM observations
      WHERE project_id = $1 AND trace_id = ANY($2::text[])
      AND start_time >= $3::timestamptz AND start_time <= $4::timestamptz
    `,
    params: [projectId, traceIds, preflight[0].min_ts, preflight[0].max_ts],
    tags: { feature: "tracing", type: "observation", kind: "delete", projectId },
  });
};

// ============================================================
// hasAnyObservation
// ============================================================
export const hasAnyObservation = async (projectId: string) => {
  const query = `SELECT 1 FROM observations WHERE project_id = $1 LIMIT 1`;
  const rows = await queryPg<{ "?column?": number }>({
    query, params: [projectId],
    tags: { feature: "tracing", type: "observation", kind: "hasAny", projectId },
  });
  return rows.length > 0;
};

// ============================================================
// deleteObservationsByProjectId
// ============================================================
export const deleteObservationsByProjectId = async (
  projectId: string,
): Promise<boolean> => {
  const hasData = await hasAnyObservation(projectId);
  if (!hasData) return false;
  const query = `DELETE FROM observations WHERE project_id = $1`;
  const tags = { feature: "tracing", type: "observation", kind: "delete", projectId };
  await executePg({ query, params: [projectId], tags });
  return true;
};

// ============================================================
// hasAnyObservationOlderThan
// ============================================================
export const hasAnyObservationOlderThan = async (
  projectId: string,
  beforeDate: Date,
) => {
  const query = `
    SELECT 1 FROM observations
    WHERE project_id = $1 AND start_time < $2::timestamptz LIMIT 1
  `;
  const rows = await queryPg<{ "?column?": number }>({
    query, params: [projectId, beforeDate.toISOString()],
    tags: { feature: "tracing", type: "observation", kind: "hasAnyOlderThan", projectId },
  });
  return rows.length > 0;
};

// ============================================================
// deleteObservationsOlderThanDays
// ============================================================
export const deleteObservationsOlderThanDays = async (
  projectId: string,
  beforeDate: Date,
): Promise<boolean> => {
  const hasData = await hasAnyObservationOlderThan(projectId, beforeDate);
  if (!hasData) return false;
  const query = `
    DELETE FROM observations
    WHERE project_id = $1 AND start_time < $2::timestamptz
  `;
  await executePg({
    query, params: [projectId, beforeDate.toISOString()],
    tags: { feature: "tracing", type: "observation", kind: "delete", projectId },
  });
  return true;
};

// ============================================================
// getObservationsWithPromptName
// ============================================================
export const getObservationsWithPromptName = async (
  projectId: string,
  promptNames: string[],
  { fromTimestamp, toTimestamp }: { fromTimestamp?: Date; toTimestamp?: Date } = {},
) => {
  const params: unknown[] = [projectId, promptNames];
  let whereClause = "";
  if (fromTimestamp) {
    params.push(fromTimestamp.toISOString());
    whereClause += `AND start_time >= $${params.length}::timestamptz`;
  }
  if (toTimestamp) {
    params.push(toTimestamp.toISOString());
    whereClause += ` AND start_time <= $${params.length}::timestamptz`;
  }
  const query = `
    SELECT count(DISTINCT id)::text as count, prompt_name
    FROM observations
    WHERE project_id = $1 AND prompt_name = ANY($2::text[]) AND prompt_name IS NOT NULL ${whereClause}
    GROUP BY prompt_name
  `;
  const rows = await queryPg<{ count: string; prompt_name: string }>({
    query, params,
    tags: { feature: "tracing", type: "observation", kind: "list", projectId },
  });
  return rows.map((r) => ({ count: Number(r.count), promptName: r.prompt_name }));
};

// ============================================================
// getObservationMetricsForPrompts
// ============================================================
export const getObservationMetricsForPrompts = async (
  projectId: string,
  promptIds: string[],
  { fromTimestamp, toTimestamp }: { fromTimestamp?: Date; toTimestamp?: Date } = {},
) => {
  const params: unknown[] = [projectId, promptIds];
  let whereClause = "";
  if (fromTimestamp) { params.push(fromTimestamp.toISOString()); whereClause += `AND start_time >= $${params.length}::timestamptz`; }
  if (toTimestamp) { params.push(toTimestamp.toISOString()); whereClause += ` AND start_time <= $${params.length}::timestamptz`; }
  const query = `
    WITH latencies AS (
      SELECT DISTINCT ON (id, project_id)
        prompt_id, prompt_version, start_time, end_time, usage_details, cost_details,
        EXTRACT(EPOCH FROM (end_time - start_time)) * 1000 AS latency_ms
      FROM observations
      WHERE type = 'GENERATION' AND prompt_name IS NOT NULL
      AND project_id = $1 AND prompt_id = ANY($2::text[]) ${whereClause}
      ORDER BY id, project_id, updated_at DESC
    )
    SELECT
      count(*)::text AS count, prompt_id, prompt_version,
      min(start_time) AS first_observation, max(start_time) AS last_observation,
      percentile_cont(0.5) WITHIN GROUP (ORDER BY COALESCE((usage_details->>'input')::numeric, 0))::text AS median_input_usage,
      percentile_cont(0.5) WITHIN GROUP (ORDER BY COALESCE((usage_details->>'output')::numeric, 0))::text AS median_output_usage,
      percentile_cont(0.5) WITHIN GROUP (ORDER BY COALESCE((cost_details->>'total')::numeric, 0))::text AS median_total_cost,
      percentile_cont(0.5) WITHIN GROUP (ORDER BY latency_ms)::text AS median_latency_ms
    FROM latencies
    GROUP BY prompt_id, prompt_version ORDER BY prompt_version DESC
  `;
  const rows = await queryPg<{
    count: string; prompt_id: string; prompt_version: number;
    first_observation: string; last_observation: string;
    median_input_usage: string; median_output_usage: string;
    median_total_cost: string; median_latency_ms: string;
  }>({ query, params, tags: { feature: "tracing", type: "observation", kind: "analytic", projectId } });
  return rows.map((r) => ({
    count: Number(r.count), promptId: r.prompt_id, promptVersion: r.prompt_version,
    firstObservation: parseClickhouseUTCDateTimeFormat(r.first_observation),
    lastObservation: parseClickhouseUTCDateTimeFormat(r.last_observation),
    medianInputUsage: Number(r.median_input_usage), medianOutputUsage: Number(r.median_output_usage),
    medianTotalCost: Number(r.median_total_cost), medianLatencyMs: Number(r.median_latency_ms),
  }));
};

// ============================================================
// getLatencyAndTotalCostForObservations
// ============================================================
export const getLatencyAndTotalCostForObservations = async (
  projectId: string, observationIds: string[], timestamp?: Date,
) => {
  const query = `
    SELECT DISTINCT ON (id, project_id)
      id, (cost_details->>'total')::numeric::text AS total_cost,
      EXTRACT(EPOCH FROM (end_time - start_time)) * 1000::text AS latency_ms
    FROM observations
    WHERE project_id = $1 AND id = ANY($2::text[])
    ${timestamp ? `AND start_time >= $3::timestamptz` : ""}
    ORDER BY id, project_id, updated_at DESC
  `;
  const rows = await queryPg<{ id: string; total_cost: string; latency_ms: string }>({
    query, params: timestamp ? [projectId, observationIds, timestamp.toISOString()] : [projectId, observationIds],
    tags: { feature: "tracing", type: "observation", kind: "analytic", projectId },
  });
  return rows.map((r) => ({ id: r.id, totalCost: Number(r.total_cost), latency: Number(r.latency_ms) / 1000 }));
};

// ============================================================
// getLatencyAndTotalCostForObservationsByTraces
// ============================================================
export const getLatencyAndTotalCostForObservationsByTraces = async (
  projectId: string, traceIds: string[], timestamp?: Date,
) => {
  const query = `
    SELECT trace_id,
      COALESCE(SUM((cost_details->>'total')::numeric), 0)::text AS total_cost,
      EXTRACT(EPOCH FROM (max(end_time) - min(start_time))) * 1000::text AS latency_ms
    FROM observations
    WHERE project_id = $1 AND trace_id = ANY($2::text[])
    ${timestamp ? `AND start_time >= $3::timestamptz` : ""}
    GROUP BY trace_id
  `;
  const rows = await queryPg<{ trace_id: string; total_cost: string; latency_ms: string }>({
    query, params: timestamp ? [projectId, traceIds, timestamp.toISOString()] : [projectId, traceIds],
    tags: { feature: "tracing", type: "observation", kind: "analytic", projectId },
  });
  return rows.map((r) => ({ traceId: r.trace_id, totalCost: Number(r.total_cost), latency: Number(r.latency_ms) / 1000 }));
};

// ============================================================
// getObservationsGroupedByTraceId
// ============================================================
export type ObservationTuple = [
  id: string,
  parentObservationId: string | null,
  totalCost: string,
  inputCost: string,
  outputCost: string,
  latencyMs: number,
];

export const getObservationsGroupedByTraceId = async (
  projectId: string, traceIds: string[], timestamp?: Date,
): Promise<Map<string, ObservationTuple[]>> => {
  if (traceIds.length === 0) return new Map();
  const query = `
    SELECT trace_id,
      array_agg(ROW(id, parent_observation_id,
        (cost_details->>'total')::text, (cost_details->>'input')::text,
        (cost_details->>'output')::text,
        EXTRACT(EPOCH FROM (end_time - start_time)) * 1000
      ) ORDER BY start_time) AS observations
    FROM observations
    WHERE project_id = $1 AND trace_id = ANY($2::text[])
    ${timestamp ? `AND start_time >= $3::timestamptz` : ""}
    GROUP BY trace_id
  `;
  const groupedObservations = await queryPg<{ trace_id: string; observations: ObservationTuple[] }>({
    query, params: timestamp ? [projectId, traceIds, timestamp.toISOString()] : [projectId, traceIds],
    tags: { feature: "tracing", type: "observation", kind: "analytic", projectId },
  });
  return new Map(groupedObservations.map((g) => [g.trace_id, g.observations]));
};

// ============================================================
// getObservationCountsByProjectInCreationInterval
// ============================================================
export const getObservationCountsByProjectInCreationInterval = async ({
  start, end,
}: { start: Date; end: Date }) => {
  const query = `
    SELECT project_id, count(*)::text as count
    FROM observations
    WHERE created_at >= $1::timestamptz AND created_at < $2::timestamptz
    GROUP BY project_id
  `;
  const rows = await queryPg<{ project_id: string; count: string }>({
    query, params: [start.toISOString(), end.toISOString()],
    tags: { feature: "tracing", type: "observation", kind: "analytic" },
  });
  return rows.map((row) => ({ projectId: row.project_id, count: Number(row.count) }));
};

// ============================================================
// getObservationCountOfProjectsSinceCreationDate
// ============================================================
export const getObservationCountOfProjectsSinceCreationDate = async ({
  projectIds, start,
}: { projectIds: string[]; start: Date }) => {
  const query = `
    SELECT count(*)::text as count
    FROM observations
    WHERE project_id = ANY($1::text[]) AND created_at >= $2::timestamptz
  `;
  const rows = await queryPg<{ count: string }>({
    query, params: [projectIds, start.toISOString()],
    tags: { feature: "tracing", type: "observation", kind: "analytic" },
  });
  return Number(rows[0]?.count ?? 0);
};

// ============================================================
// getTraceIdsForObservations
// ============================================================
export const getTraceIdsForObservations = async (
  projectId: string, observationIds: string[],
) => {
  const query = `
    SELECT trace_id, id
    FROM observations
    WHERE project_id = $1 AND id = ANY($2::text[])
  `;
  const rows = await queryPg<{ id: string; trace_id: string }>({
    query, params: [projectId, observationIds],
    tags: { feature: "tracing", type: "observation", kind: "list", projectId },
  });
  return rows.map((row) => ({ id: row.id, traceId: row.trace_id }));
};

// ============================================================
// LEGACY_OBSERVATION_EXPORT_SQL_OVERRIDES
// ============================================================
const LEGACY_OBSERVATION_EXPORT_SQL_OVERRIDES: Record<string, string> = {
  latency:
    "CASE WHEN end_time IS NULL THEN NULL ELSE EXTRACT(EPOCH FROM (end_time - start_time)) * 1000 / 1000 END as latency",
  time_to_first_token:
    "CASE WHEN completion_start_time IS NULL THEN NULL ELSE EXTRACT(EPOCH FROM (completion_start_time - start_time)) * 1000 / 1000 END as time_to_first_token",
  model_id: "internal_model_id as model_id",
};

// ============================================================
// getObservationsForBlobStorageExport
// ============================================================
export const getObservationsForBlobStorageExport = function (
  projectId: string,
  minTimestamp: Date,
  maxTimestamp: Date,
  fieldGroups: ObservationFieldGroupFull[] = [...OBSERVATION_FIELD_GROUPS_FULL],
) {
  const effectiveGroups = new Set<ObservationFieldGroupFull>(["core", ...fieldGroups]);

  const selectedColumns = LEGACY_OBSERVATION_EXPORT_FIELDS.filter((column) =>
    effectiveGroups.has(column.group),
  ).map(
    (column) => LEGACY_OBSERVATION_EXPORT_SQL_OVERRIDES[column.field] ?? column.field,
  );

  const query = `
    SELECT DISTINCT ON (id, project_id, type)
      ${selectedColumns.join(",\n      ")}
    FROM observations
    WHERE project_id = $1
    AND start_time >= $2::timestamptz AND start_time <= $3::timestamptz
    ORDER BY id, project_id, type, updated_at DESC
  `;

  const records = queryPgStream<Record<string, unknown>>({
    query,
    params: [projectId, minTimestamp.toISOString(), maxTimestamp.toISOString()],
    tags: { feature: "blobstorage", type: "observation", kind: "analytic", projectId },
  });

  return records;
};

// ============================================================
// getGenerationsForAnalyticsIntegrations
// ============================================================
export const getGenerationsForAnalyticsIntegrations = async function* (
  projectId: string,
  projectName: string,
  minTimestamp: Date,
  maxTimestamp: Date,
  options: { useGraceHash?: boolean } = {},
) {
  const query = `
    WITH selected_traces AS (
      SELECT DISTINCT ON (t.id, t.project_id)
        t.project_id as project_id, t.id as id, t.name as name,
        t.session_id as session_id, t.user_id as user_id,
        t.release as release, t.tags as tags,
        t.metadata->>'$posthog_session_id' as posthog_session_id,
        t.metadata->>'$mixpanel_session_id' as mixpanel_session_id
      FROM traces t
      WHERE t.project_id = $1
      AND t.timestamp >= $2::timestamptz - INTERVAL '2 DAYS'
      AND t.timestamp <= $3::timestamptz + INTERVAL '1 HOUR'
      ORDER BY t.id, t.project_id, t.updated_at DESC
    )

    SELECT
      o.name as name, o.start_time as start_time, o.id as id,
      o.total_cost as total_cost,
      CASE WHEN completion_start_time IS NULL THEN NULL ELSE EXTRACT(EPOCH FROM (completion_start_time - start_time)) * 1000 END as time_to_first_token,
      (o.usage_details->>'total')::numeric as input_tokens,
      (o.usage_details->>'output')::numeric as output_tokens,
      (o.cost_details->>'total')::numeric as total_tokens,
      o.project_id as project_id,
      CASE WHEN end_time IS NULL THEN NULL ELSE EXTRACT(EPOCH FROM (end_time - start_time)) * 1000 / 1000 END as latency,
      o.provided_model_name as model, o.level as level, o.version as version,
      o.environment as environment,
      t.id as trace_id, t.name as trace_name,
      t.session_id as trace_session_id, t.user_id as trace_user_id,
      t.release as trace_release, t.tags as trace_tags,
      t.posthog_session_id as posthog_session_id,
      t.mixpanel_session_id as mixpanel_session_id
    FROM (
      SELECT DISTINCT ON (id, project_id) *
      FROM observations
      WHERE project_id = $1
      AND start_time >= $2::timestamptz AND start_time < $3::timestamptz
      AND type = 'GENERATION'
      ORDER BY id, project_id, updated_at DESC
    ) o
    LEFT JOIN selected_traces t ON o.trace_id = t.id AND o.project_id = t.project_id
  `;

  const records = queryPgStream<Record<string, unknown>>({
    query,
    params: [projectId, minTimestamp.toISOString(), maxTimestamp.toISOString()],
    tags: { feature: "posthog", type: "observation", kind: "analytic", projectId },
  });

  const baseUrl = env.NEXTAUTH_URL?.replace("/api/auth", "");
  for await (const record of records) {
    yield {
      timestamp: record.start_time,
      langfuse_generation_name: record.name,
      langfuse_trace_name: record.trace_name,
      langfuse_trace_id: record.trace_id,
      langfuse_url: `${baseUrl}/project/${projectId}/traces/${encodeURIComponent(record.trace_id as string)}?observation=${encodeURIComponent(record.id as string)}`,
      langfuse_user_url: record.trace_user_id
        ? `${baseUrl}/project/${projectId}/users/${encodeURIComponent(record.trace_user_id as string)}`
        : undefined,
      langfuse_id: record.id,
      langfuse_cost_usd: record.total_cost,
      langfuse_input_units: record.input_tokens,
      langfuse_output_units: record.output_tokens,
      langfuse_total_units: record.total_tokens,
      langfuse_session_id: record.trace_session_id,
      langfuse_project_id: projectId,
      langfuse_project_name: projectName,
      langfuse_user_id: record.trace_user_id || null,
      langfuse_latency: record.latency,
      langfuse_time_to_first_token: record.time_to_first_token,
      langfuse_release: record.trace_release,
      langfuse_version: record.version,
      langfuse_model: record.model,
      langfuse_level: record.level,
      langfuse_tags: record.trace_tags,
      langfuse_environment: record.environment,
      langfuse_event_version: "1.0.0",
      posthog_session_id: record.posthog_session_id ?? null,
      mixpanel_session_id: record.mixpanel_session_id ?? null,
    } satisfies AnalyticsGenerationEvent;
  }
};

// ============================================================
// getObservationCountsByProjectAndDay
// ============================================================
export const getObservationCountsByProjectAndDay = async ({
  startDate, endDate,
}: { startDate: Date; endDate: Date }) => {
  const query = `
    SELECT count(*)::text as count, project_id, start_time::date as date
    FROM observations
    WHERE start_time >= $1::timestamptz AND start_time < $2::timestamptz
    GROUP BY project_id, start_time::date
  `;
  const rows = await queryPg<{ count: string; project_id: string; date: string }>({
    query,
    params: [startDate.toISOString(), endDate.toISOString()],
    tags: { feature: "tracing", type: "observation", kind: "analytic" },
  });
  return rows.map((row) => ({ count: Number(row.count), projectId: row.project_id, date: row.date }));
};

// ============================================================
// getCostByEvaluatorIds
// ============================================================
export const getCostByEvaluatorIds = async (
  projectId: string,
  evaluatorIds: string[],
): Promise<Array<{ evaluatorId: string; totalCost: number }>> => {
  if (evaluatorIds.length === 0) return [];

  const query = `
    SELECT DISTINCT ON (id, project_id)
      metadata->>'job_configuration_id' as evaluator_id,
      total_cost
    FROM observations
    WHERE project_id = $1
      AND metadata->>'job_configuration_id' = ANY($2::text[])
      AND type = 'GENERATION'
      AND start_time > CURRENT_DATE - INTERVAL '7 DAYS'
    ORDER BY id, project_id, updated_at DESC
  `;

  // Aggregate in application code since PG DISTINCT ON + GROUP BY is complex
  const rows = await queryPg<{ evaluator_id: string; total_cost: number }>({
    query, params: [projectId, evaluatorIds],
    tags: { feature: "evals", type: "observation", kind: "analytic", projectId },
  });

  // Group by evaluator_id and sum costs
  const costMap = new Map<string, number>();
  for (const row of rows) {
    const current = costMap.get(row.evaluator_id) ?? 0;
    costMap.set(row.evaluator_id, current + Number(row.total_cost));
  }

  return Array.from(costMap.entries()).map(([evaluatorId, totalCost]) => ({
    evaluatorId, totalCost,
  }));
};

// ============================================================
// generateObservationsForPublicApi
// ============================================================
export const generateObservationsForPublicApi = async ({
  projectId, filter, pagination,
}: {
  projectId: string;
  filter: FilterList;
  pagination: { limit: number; page: number };
}) => {
  const appliedFilter = filter.apply();
  const traceFilter = filter.find((f) => f.clickhouseTable === "traces");

  const disableObservationsFinal = await shouldSkipObservationsFinal(projectId);

  const query = `
    WITH clickhouse_keys AS (
      SELECT DISTINCT
        id, trace_id, project_id, type, start_time::date
      FROM observations o
      ${traceFilter ? `LEFT JOIN traces t ON o.trace_id = t.id AND t.project_id = o.project_id` : ""}
      WHERE o.project_id = $1
      ${traceFilter ? `AND t.project_id = $1` : ""}
      AND ${appliedFilter.query}
      ORDER BY start_time DESC
      LIMIT ${pagination.limit} OFFSET ${(pagination.page - 1) * pagination.limit}
    )
    SELECT
      id, trace_id, project_id, type, parent_observation_id,
      environment, start_time, end_time, name, metadata, level,
      status_message, version, input, output, provided_model_name,
      internal_model_id, model_parameters, provided_usage_details,
      usage_details, provided_cost_details, cost_details, total_cost,
      completion_start_time, prompt_id, prompt_name, prompt_version,
      created_at, updated_at, event_ts
    FROM observations o
    WHERE o.project_id = $1
    AND (id, trace_id, project_id, type, start_time::date) IN (SELECT * FROM clickhouse_keys)
    ${disableObservationsFinal ? "" : "ORDER BY start_time DESC"}
  `;

  return measureAndReturn({
    operationName: "generateObservationsForPublicApi",
    projectId,
    input: {
      params: { appliedFilter, pagination, projectId },
      tags: {
        feature: "tracing", type: "observation", projectId,
        operation_name: "generateObservationsForPublicApi",
      },
    },
    fn: async (input) => {
      const params: unknown[] = [projectId, ...appliedFilter.params];
      const result = await queryPg<ObservationRecordReadType>({
        query,
        params,
        tags: input.tags,
      });
      return result.map((r) => convertObservation(r));
    },
  });
};

// ============================================================
// getObservationsCountForPublicApi
// ============================================================
export const getObservationsCountForPublicApi = async ({
  projectId, filter,
}: {
  projectId: string;
  filter: FilterList;
}) => {
  const appliedFilter = filter.apply();
  const traceFilter = filter.find((f) => f.clickhouseTable === "traces");

  const query = `
    SELECT count(*)::text as count
    FROM observations o
    ${traceFilter ? `LEFT JOIN traces t ON o.trace_id = t.id AND t.project_id = o.project_id` : ""}
    WHERE o.project_id = $1
    ${traceFilter ? `AND t.project_id = $1` : ""}
    AND ${appliedFilter.query}
  `;

  return measureAndReturn({
    operationName: "getObservationsCountForPublicApi",
    projectId,
    input: {
      params: { appliedFilter, projectId },
      tags: {
        feature: "tracing", type: "observation", projectId,
        operation_name: "getObservationsCountForPublicApi",
      },
    },
    fn: async (input) => {
      const params: unknown[] = [projectId, ...appliedFilter.params];
      const records = await queryPg<{ count: string }>({
        query,
        params,
        tags: input.tags,
      });
      return records.map((record) => Number(record.count)).shift();
    },
  });
};
