import { z } from "zod";
import {
  ScoreDataTypeType,
  ScoreDomain,
  ScoreSourceType,
  AGGREGATABLE_SCORE_TYPES,
  AggregatableScoreDataType,
  ScoreByDataType,
  LISTABLE_SCORE_TYPES,
  ListableScoreDataType,
  ScoreDataTypeEnum,
} from "../../domain/scores";
import { InvalidRequestError, InternalServerError } from "../../errors";
import type { APIScoreV3 } from "../../features/scores/interfaces/api/v3/schemas";
import type { ScoreFieldGroupV3 } from "../../features/scores/interfaces/api/v3/endpoints";
import { filterAndValidateV3GetScoreList } from "../../features/scores/interfaces/api/v3/validation";
import {
  queryPg,
  executePg,
  queryPgStream,
  toPgTimestamp,
  pgCompliantRandomCharacters,
} from "./pg";
import {
  StringFilter as PgStringFilter,
  DateTimeFilter as PgDateTimeFilter,
  NumberFilter as PgNumberFilter,
  ArrayOptionsFilter as PgArrayOptionsFilter,
  BooleanFilter as PgBooleanFilter,
  NullFilter as PgNullFilter,
  StringOptionsFilter as PgStringOptionsFilter,
  type Filter,
  type PgFilterResult,
  applyFilterList,
} from "../queries/pg-sql/pg-filter";
import { FilterCondition, FilterState, TimeFilter } from "../../types";
import { filterOperators } from "../../interfaces/filters";
import { OrderByState } from "../../interfaces/orderBy";
import { orderByToPgSql } from "../queries/pg-sql/orderby-factory";
import {
  dashboardColumnDefinitions,
  scoresTableUiColumnDefinitions,
  scoresTableUiColumnDefinitionsFromEvents,
} from "../tableMappings";
import {
  convertScoreAggregation,
  convertClickhouseScoreToDomain,
  ScoreAggregation,
} from "./scores_converters";
import { SCORE_TO_TRACE_OBSERVATIONS_INTERVAL } from "./constants";
import { ScoreRecordReadType } from "./definitions";
import { env } from "../../env";
import { _handleGetScoreById, _handleGetScoresByIds } from "./scores-utils";
import { parseMetadataCHRecordToDomain } from "../utils/metadata_conversion";
import type { AnalyticsScoreEvent } from "../analytics-integrations/types";
import { recordDistribution } from "../instrumentation";
import { logger } from "../logger";
import { prisma } from "../../db";
import { scoresColumnsTableUiColumnDefinitions } from "../tableMappings/mapScoresColumnsTable";
import { scoresTableCols } from "../../tableDefinitions/scoresTable";
import {
  findUiColumnMapping,
  matchesUiColumnMapping,
} from "../../tableDefinitions";

// ─── PG FilterList ────────────────────────────────────────────────────────────
// Wraps PG Filter[] with the same API as the old ClickHouse FilterList class,
// but internally uses PG positional-parameter apply().
class PgFilterList {
  constructor(private filters: Filter[] = []) {}

  push(...filter: Filter[]) {
    this.filters.push(...filter);
  }

  find(predicate: (f: Filter) => boolean) {
    return this.filters.find(predicate);
  }

  filter(predicate: (f: Filter) => boolean) {
    return new PgFilterList(this.filters.filter(predicate));
  }

  some(predicate: (f: Filter) => boolean) {
    return this.filters.some(predicate);
  }

  forEach(callback: (f: Filter) => void) {
    this.filters.forEach(callback);
  }

  length() {
    return this.filters.length;
  }

  apply(): PgFilterResult {
    if (this.filters.length === 0) {
      return { query: "", params: [] };
    }
    return applyFilterList(this.filters);
  }
}

// ─── PG getProjectIdDefaultFilter ─────────────────────────────────────────────
function getProjectIdDefaultFilter(
  projectId: string,
  _opts?: { tracesPrefix?: string },
): { scoresFilter: PgFilterList; tracesFilter: PgFilterList } {
  const scoresFilter = new PgFilterList([
    new PgStringFilter({
      clickhouseTable: "scores",
      field: "project_id",
      operator: "=",
      value: projectId,
      tablePrefix: "s",
    }),
  ]);
  const tracesFilter = new PgFilterList([
    new PgStringFilter({
      clickhouseTable: "traces",
      field: "project_id",
      operator: "=",
      value: projectId,
      tablePrefix: "t",
    }),
  ]);
  return { scoresFilter, tracesFilter };
}

// ─── createFilterFromFilterState -> PG adapter ─────────────────────────────────
// PG factory returns filters without proper column mapping; we rebuild with
// column definitions for correct table prefixes and field names.

/**
 * Parse a ClickHouse-style column reference like 's."trace_id"' or '"timestamp"'
 * into { tablePrefix, fieldName } for PG filter construction.
 */
function parseClickhouseSelect(chSelect: string): { tablePrefix?: string; fieldName: string } {
  // Match patterns like: s."trace_id", dri."dataset_id", "timestamp", or plain field_name
  const match = chSelect.match(/^(?:(\w+)\.")?(.+)"$/) || chSelect.match(/^(?:(\w+)\.)?([^"]+)$/);
  if (match) {
    return { tablePrefix: match[1] || undefined, fieldName: match[2] };
  }
  return { fieldName: chSelect };
}

function createFilterFromFilterStatePg(
  filter: FilterState,
  columnMapping?: { uiTableId: string; clickhouseTable: string; clickhouseSelect: string; queryPrefix?: string }[],
  _columnDefinitions?: unknown[],
): Filter[] {
  const results: Filter[] = [];
  for (const f of filter) {
    const mapping = columnMapping?.find((m) => m.uiTableId === f.column);
    const tableName = mapping?.clickhouseTable ?? "scores";
    const rawField = mapping?.clickhouseSelect ?? f.column;
    const prefix = mapping?.queryPrefix;
    // Parse field to extract table prefix and unquoted field name
    const { tablePrefix: parsedPrefix, fieldName } = parseClickhouseSelect(rawField);
    const effectivePrefix = prefix ?? parsedPrefix;

    switch (f.type) {
      case "string":
        results.push(
          new PgStringFilter({
            clickhouseTable: tableName,
            field: fieldName,
            operator: f.operator as (typeof filterOperators)["string"][number],
            value: f.value,
            tablePrefix: effectivePrefix,
          }),
        );
        break;
      case "datetime":
        results.push(
          new PgDateTimeFilter({
            clickhouseTable: tableName,
            field: fieldName,
            operator: f.operator as (typeof filterOperators)["datetime"][number],
            value: new Date(f.value),
            tablePrefix: effectivePrefix,
          }),
        );
        break;
      case "number":
        results.push(
          new PgNumberFilter({
            clickhouseTable: tableName,
            field: fieldName,
            operator: f.operator as (typeof filterOperators)["number"][number],
            value: f.value,
            tablePrefix: effectivePrefix,
          }),
        );
        break;
      case "arrayOptions":
        results.push(
          new PgArrayOptionsFilter({
            clickhouseTable: tableName,
            field: fieldName,
            operator: f.operator as (typeof filterOperators)["arrayOptions"][number],
            values: f.value,
            tablePrefix: effectivePrefix,
          }),
        );
        break;
      case "boolean":
        results.push(
          new PgBooleanFilter({
            clickhouseTable: tableName,
            field: fieldName,
            operator: f.operator as (typeof filterOperators)["boolean"][number],
            value: f.value,
            tablePrefix: effectivePrefix,
          }),
        );
        break;
      case "null":
        results.push(
          new PgNullFilter({
            clickhouseTable: tableName,
            field: fieldName,
            operator: f.operator as (typeof filterOperators)["null"][number],
            tablePrefix: effectivePrefix,
          }),
        );
        break;
      case "stringOptions":
        results.push(
          new PgStringOptionsFilter({
            clickhouseTable: tableName,
            field: fieldName,
            operator: f.operator as (typeof filterOperators)["stringOptions"][number],
            values: f.value,
            tablePrefix: effectivePrefix,
          }),
        );
        break;
    }
  }
  return results;
}

// ─── helper: build trace metadata CTE inline (PG dialect) ─────────────────────
function buildTracesMetadataCte(projectId: string): {
  query: string;
  params: unknown[];
} {
  return {
    query: `SELECT e.trace_id AS id, e.trace_name AS name, e.user_id AS user_id, e.tags AS tags
FROM events e
WHERE e.project_id = $1
AND e.trace_name <> ''
AND e.is_deleted = FALSE
`,
    params: [projectId],
  };
}

// ─── helper: build experiment events CTE inline (PG dialect) ──────────────────
function buildExperimentEventsCte(
  projectId: string,
  experimentIds: string[],
): { query: string; params: unknown[] } {
  return {
    query: `SELECT e.project_id, e.experiment_id, e.trace_id
FROM events e
WHERE e.project_id = $1
AND e.experiment_id = ANY($2::text[])
AND e.is_deleted = FALSE
`,
    params: [projectId, experimentIds],
  };
}

// ─── helper: build experiment trace IDs CTE inline (PG dialect) ───────────────
function buildExperimentTraceIdsCte(projectId: string): {
  query: string;
  params: unknown[];
} {
  return {
    query: `SELECT e.project_id, e.trace_id
FROM events e
WHERE e.project_id = $1
AND e.is_deleted = FALSE
`,
    params: [projectId],
  };
}

// ─── helper: measureAndReturn PG equivalent ───────────────────────────────────
async function measureAndReturn<T, I>(opts: {
  operationName: string;
  projectId: string;
  input: I;
  fn: (input: I) => Promise<T>;
}): Promise<T> {
  return opts.fn(opts.input);
}

const FILTER_OPTION_SCORE_NAME_LIMIT = 200;
const FILTER_OPTION_CATEGORICAL_VALUE_LIMIT = 20;

export const searchExistingAnnotationScore = async (
  projectId: string,
  observationId: string | null,
  traceId: string | null,
  sessionId: string | null,
  name: string | undefined,
  configId: string | undefined,
  dataType: ScoreDataTypeType,
) => {
  if (!name && !configId) {
    throw new Error("Either name or configId (or both) must be provided.");
  }

  const params: unknown[] = [projectId, dataType];
  let paramIdx = 3;

  const traceIdClause = traceId
    ? `AND s.trace_id = $${paramIdx++}`
    : "AND s.trace_id IS NULL";
  if (traceId) params.push(traceId);

  const obsClause = observationId
    ? `AND s.observation_id = $${paramIdx++}`
    : "AND s.observation_id IS NULL";
  if (observationId) params.push(observationId);

  const sessClause = sessionId
    ? `AND s.session_id = $${paramIdx++}`
    : "AND s.session_id IS NULL";
  if (sessionId) params.push(sessionId);

  const nameOrClause = name ? `OR s.name = $${paramIdx++}` : "";
  if (name) params.push(name);

  const cfgOrClause = configId ? `OR s.config_id = $${paramIdx++}` : "";
  if (configId) params.push(configId);

  const query = `
    SELECT * FROM (
      SELECT DISTINCT ON (s.id, s.project_id) *
      FROM scores s
      WHERE s.project_id = $1
      AND s.source = 'ANNOTATION'
      AND s.data_type = $2
      ${traceIdClause}
      ${obsClause}
      ${sessClause}
      AND (
        FALSE
        ${nameOrClause}
        ${cfgOrClause}
      )
      ORDER BY s.id, s.project_id, s.updated_at DESC
    ) sub
    LIMIT 1
  `;

  const rows = await queryPg<ScoreRecordReadType>({
    query,
    params,
    tags: {
      feature: "tracing",
      type: "score",
      kind: "list",
      projectId,
    },
  });
  return rows.map((row) => convertClickhouseScoreToDomain(row)).shift();
};

export const getScoreById = async ({
  projectId,
  scoreId,
  source,
}: {
  projectId: string;
  scoreId: string;
  source?: ScoreSourceType;
}): Promise<ScoreDomain | undefined> => {
  return _handleGetScoreById({
    projectId,
    scoreId,
    source,
    scoreScope: "all",
  });
};

export const getScoresByIds = async (
  projectId: string,
  scoreId: string[],
  source?: ScoreSourceType,
): Promise<ScoreDomain[]> => {
  return _handleGetScoresByIds({
    projectId,
    scoreId,
    source,
    scoreScope: "all",
    dataTypes: LISTABLE_SCORE_TYPES,
  });
};

/**
 * Accepts a score in a Clickhouse-ready format.
 * id, project_id, name, and timestamp must always be provided.
 */
export const upsertScore = async (score: Partial<ScoreRecordReadType>) => {
  if (!["id", "project_id", "name", "timestamp"].every((key) => key in score)) {
    throw new Error("Identifier fields must be provided to upsert Score.");
  }
  const s = score as ScoreRecordReadType;
  const params = [
    s.id,
    s.project_id,
    s.timestamp,
    s.name,
    s.value,
    s.source ?? null,
    s.data_type ?? null,
    s.environment ?? null,
    s.trace_id ?? null,
    s.session_id ?? null,
    s.observation_id ?? null,
    s.dataset_run_id ?? null,
    s.comment ?? null,
    s.author_user_id ?? null,
    s.config_id ?? null,
    s.string_value ?? null,
    s.long_string_value ?? null,
    s.queue_id ?? null,
    s.execution_trace_id ?? null,
    s.created_at ?? null,
    s.updated_at ?? null,
    s.event_ts ?? null,
    s.is_deleted ?? 0,
    JSON.stringify(s.metadata ?? {}),
  ];

  await executePg({
    query: `
      INSERT INTO scores (
        id, project_id, timestamp, name, value, source, data_type,
        environment, trace_id, session_id, observation_id, dataset_run_id,
        comment, author_user_id, config_id, string_value, long_string_value,
        queue_id, execution_trace_id, created_at, updated_at, event_ts,
        is_deleted, metadata
      ) VALUES (
        $1, $2, $3::timestamptz, $4, $5, $6, $7,
        $8, $9, $10, $11, $12,
        $13, $14, $15, $16, $17,
        $18, $19, $20::timestamptz, $21::timestamptz, $22::timestamptz,
        $23, $24::jsonb
      )
      ON CONFLICT (id, project_id) DO UPDATE SET
        timestamp = EXCLUDED.timestamp,
        name = EXCLUDED.name,
        value = EXCLUDED.value,
        source = EXCLUDED.source,
        data_type = EXCLUDED.data_type,
        environment = EXCLUDED.environment,
        trace_id = EXCLUDED.trace_id,
        session_id = EXCLUDED.session_id,
        observation_id = EXCLUDED.observation_id,
        dataset_run_id = EXCLUDED.dataset_run_id,
        comment = EXCLUDED.comment,
        author_user_id = EXCLUDED.author_user_id,
        config_id = EXCLUDED.config_id,
        string_value = EXCLUDED.string_value,
        long_string_value = EXCLUDED.long_string_value,
        queue_id = EXCLUDED.queue_id,
        execution_trace_id = EXCLUDED.execution_trace_id,
        created_at = EXCLUDED.created_at,
        updated_at = EXCLUDED.updated_at,
        event_ts = EXCLUDED.event_ts,
        is_deleted = EXCLUDED.is_deleted,
        metadata = EXCLUDED.metadata
    `,
    params,
    tags: {
      feature: "tracing",
      type: "score",
      kind: "upsert",
      projectId: score.project_id ?? "",
    },
  });
};

export type GetScoresForTracesProps<
  ExcludeMetadata extends boolean,
  IncludeHasMetadata extends boolean,
> = {
  projectId: string;
  traceIds: string[];
  level?: "trace" | "observation" | "all";
  timestamp?: Date;
  limit?: number;
  offset?: number;
  excludeMetadata?: ExcludeMetadata;
  includeHasMetadata?: IncludeHasMetadata;
};

type GetScoresForSessionsProps<
  ExcludeMetadata extends boolean,
  IncludeHasMetadata extends boolean,
> = {
  projectId: string;
  sessionIds: string[];
  limit?: number;
  offset?: number;
  excludeMetadata?: ExcludeMetadata;
  includeHasMetadata?: IncludeHasMetadata;
};

type GetScoresForExperimentsProps<
  ExcludeMetadata extends boolean,
  IncludeHasMetadata extends boolean,
> = {
  projectId: string;
  runIds: string[];
  limit?: number;
  offset?: number;
  excludeMetadata?: ExcludeMetadata;
  includeHasMetadata?: IncludeHasMetadata;
};

const formatMetadataSelect = (
  excludeMetadata: boolean,
  includeHasMetadata: boolean,
) => {
  return [
    !excludeMetadata ? "*" : [
      // PG-only: only columns that exist in the PG scores table
      "s.id", "s.timestamp", "s.project_id",
      "s.trace_id", "s.observation_id",
      "s.name", "s.value", "s.source", "s.comment", "s.author_user_id",
      "s.config_id", "s.data_type", "s.string_value",
      "s.queue_id", "s.created_at", "s.updated_at",
      // PG-mapped: columns that don't exist in PG → return defaults
      "'default' as environment",
      "NULL::text as session_id",
      "NULL::text as dataset_run_id",
      "NULL::text as long_string_value",
      "NULL::text as execution_trace_id",
      "s.updated_at as event_ts",
      "0 as is_deleted",
    ].join(", "),
    includeHasMetadata
      ? "0 as has_metadata"
      : null,
  ]
    .filter((s) => s != null)
    .join(", ");
};

export const getScoresForSessions = async <
  ExcludeMetadata extends boolean,
  IncludeHasMetadata extends boolean,
>(
  props: GetScoresForSessionsProps<ExcludeMetadata, IncludeHasMetadata>,
) => {
  const {
    projectId,
    sessionIds,
    limit,
    offset,
    excludeMetadata = false,
    includeHasMetadata = false,
  } = props;

  const select = formatMetadataSelect(excludeMetadata, includeHasMetadata);
  const params: unknown[] = [projectId, sessionIds, LISTABLE_SCORE_TYPES];
  let paramIdx = 4;

  let limitClause = "";
  if (limit !== undefined && offset !== undefined) {
    limitClause = `LIMIT $${paramIdx++} OFFSET $${paramIdx++}`;
    params.push(limit, offset);
  }

  const query = `
    SELECT * FROM (
      SELECT DISTINCT ON (s.id, s.project_id)
        ${select}
      FROM scores s
      WHERE s.project_id = $1
      AND s.session_id = ANY($2::text[])
      AND s.data_type::text = ANY($3::text[])
      ORDER BY s.id, s.project_id, s.updated_at DESC
    ) sub
    ${limitClause}
  `;

  const rows = await queryPg<ScoreRecordReadType>({
    query,
    params,
    tags: {
      feature: "sessions",
      type: "score",
      kind: "list",
      projectId,
    },
  });

  const includeMetadataPayload = excludeMetadata ? false : true;
  return rows.map((row) =>
    convertClickhouseScoreToDomain(row, includeMetadataPayload),
  );
};

export const getScoresForExperiments = async <
  ExcludeMetadata extends boolean,
  IncludeHasMetadata extends boolean,
>(
  props: GetScoresForExperimentsProps<ExcludeMetadata, IncludeHasMetadata>,
) => {
  const {
    projectId,
    runIds,
    limit,
    offset,
    excludeMetadata = false,
    includeHasMetadata = false,
  } = props;

  const select = formatMetadataSelect(excludeMetadata, includeHasMetadata);
  const params: unknown[] = [projectId, runIds, AGGREGATABLE_SCORE_TYPES];
  let paramIdx = 4;

  let limitClause = "";
  if (limit !== undefined && offset !== undefined) {
    limitClause = `LIMIT $${paramIdx++} OFFSET $${paramIdx++}`;
    params.push(limit, offset);
  }

  const query = `
    SELECT * FROM (
      SELECT DISTINCT ON (s.id, s.project_id)
        ${select}
      FROM scores s
      WHERE s.project_id = $1
      AND s.dataset_run_id = ANY($2::text[])
      AND s.data_type::text = ANY($3::text[])
      ORDER BY s.id, s.project_id, s.updated_at DESC
    ) sub
    ${limitClause}
  `;

  const rows = await queryPg<ScoreRecordReadType>({
    query,
    params,
    tags: {
      feature: "sessions",
      type: "score",
      kind: "list",
      projectId,
    },
  });

  const includeMetadataPayload = excludeMetadata ? false : true;
  return rows.map((row) =>
    convertClickhouseScoreToDomain<ExcludeMetadata, AggregatableScoreDataType>(
      row,
      includeMetadataPayload,
    ),
  );
};

export const getTraceScoresForDatasetRuns = async (
  projectId: string,
  datasetRunIds: string[],
): Promise<Array<{ dataset_run_id: string } & any>> => {
  if (datasetRunIds.length === 0) return [];

  const query = `
    SELECT * FROM (
      SELECT DISTINCT ON (s.id, s.project_id, dri.dataset_run_id)
        s.id as id,
        s.timestamp as timestamp,
        s.project_id as project_id,
        s.environment as environment,
        s.trace_id as trace_id,
        s.session_id as session_id,
        s.observation_id as observation_id,
        s.dataset_run_id as dataset_run_id,
        s.name as name,
        s.value as value,
        s.source as source,
        s.comment as comment,
        s.author_user_id as author_user_id,
        s.config_id as config_id,
        s.data_type as data_type,
        s.string_value as string_value,
        s.queue_id as queue_id,
        s.execution_trace_id as execution_trace_id,
        s.created_at as created_at,
        s.updated_at as updated_at,
        s.event_ts as event_ts,
        s.is_deleted as is_deleted,
        CASE WHEN s.metadata IS NOT NULL AND s.metadata::text <> '{}'::text THEN 1 ELSE 0 END AS has_metadata,
        dri.dataset_run_id as run_id
      FROM dataset_run_items dri
      JOIN scores s ON dri.trace_id = s.trace_id
        AND dri.project_id = s.project_id
      WHERE dri.project_id = $1
        AND dri.dataset_run_id = ANY($2::text[])
        AND s.project_id = $1
        AND s.data_type::text = ANY($3::text[])
      ORDER BY s.id, s.project_id, dri.dataset_run_id, s.updated_at DESC
    ) sub
  `;

  const rows = await queryPg<
    Omit<ScoreRecordReadType, "metadata"> & {
      has_metadata: 0 | 1;
      run_id: string;
    }
  >({
    query,
    params: [projectId, datasetRunIds, AGGREGATABLE_SCORE_TYPES],
    tags: {
      feature: "dataset-run-items",
      type: "trace-scores",
      kind: "list",
      projectId,
    },
  });

  const includeMetadataPayload = false;
  return rows.map((row) => ({
    ...convertClickhouseScoreToDomain(
      { ...row, metadata: {} },
      includeMetadataPayload,
    ),
    datasetRunId: row.run_id,
    hasMetadata: !!row.has_metadata,
  }));
};

export const getScoresForExperimentItems = async (
  projectId: string,
  experimentIds: string[],
): Promise<
  Array<
    ScoreByDataType<AggregatableScoreDataType> & {
      experimentId: string;
      hasMetadata: boolean;
    }
  >
> => {
  if (experimentIds.length === 0) return [];

  // Build experiment events CTE inline
  const cteQuery = buildExperimentEventsCte(projectId, experimentIds);

  const query = `
    WITH experiment_events AS (
      ${cteQuery.query}
    )
    SELECT * FROM (
      SELECT DISTINCT ON (s.id, s.project_id, e.experiment_id)
        s.id as id,
        s.timestamp as timestamp,
        s.project_id as project_id,
        s.environment as environment,
        s.trace_id as trace_id,
        s.session_id as session_id,
        s.observation_id as observation_id,
        s.dataset_run_id as dataset_run_id,
        s.name as name,
        s.value as value,
        s.source as source,
        s.comment as comment,
        s.author_user_id as author_user_id,
        s.config_id as config_id,
        s.data_type as data_type,
        s.string_value as string_value,
        s.queue_id as queue_id,
        s.execution_trace_id as execution_trace_id,
        s.created_at as created_at,
        s.updated_at as updated_at,
        s.event_ts as event_ts,
        s.is_deleted as is_deleted,
        CASE WHEN s.metadata IS NOT NULL AND s.metadata::text <> '{}'::text THEN 1 ELSE 0 END AS has_metadata,
        e.experiment_id as experiment_id
      FROM experiment_events e
      JOIN scores s ON e.trace_id = s.trace_id
        AND e.project_id = s.project_id
      WHERE s.project_id = $1
        AND s.data_type::text = ANY($2::text[])
      ORDER BY s.id, s.project_id, e.experiment_id, s.updated_at DESC
    ) sub
  `;

  const rows = await queryPg<
    Omit<ScoreRecordReadType, "metadata"> & {
      has_metadata: 0 | 1;
      experiment_id: string;
    }
  >({
    query,
    params: [
      projectId,
      AGGREGATABLE_SCORE_TYPES,
      ...cteQuery.params,
    ],
    tags: {
      feature: "experiments",
      type: "trace-scores",
      kind: "list",
      projectId,
    },
  });

  const includeMetadataPayload = false;
  return rows.map((row) => ({
    ...convertClickhouseScoreToDomain<false, AggregatableScoreDataType>(
      { ...row, metadata: {} },
      includeMetadataPayload,
    ),
    experimentId: row.experiment_id,
    hasMetadata: !!row.has_metadata,
  }));
};

const getScoresForTracesInternal = async <
  ExcludeMetadata extends boolean,
  IncludeHasMetadata extends boolean,
  DataTypes extends readonly ScoreDataTypeType[],
>(
  props: GetScoresForTracesProps<ExcludeMetadata, IncludeHasMetadata> & {
    dataTypes?: DataTypes;
  },
) => {
  const {
    projectId,
    traceIds,
    level = "all",
    timestamp,
    dataTypes,
    limit,
    offset,
    excludeMetadata = false,
    includeHasMetadata = false,
  } = props;

  const select = formatMetadataSelect(excludeMetadata, includeHasMetadata);
  const levelFilter =
    level === "trace"
      ? "AND s.observation_id IS NULL"
      : level === "observation"
        ? "AND s.observation_id IS NOT NULL"
        : "";

  const params: unknown[] = [projectId, traceIds];
  let paramIdx = 3;

  let dataTypeClause = "";
  if (dataTypes) {
    dataTypeClause = `AND s.data_type::text = ANY($${paramIdx++}::text[])`;
    params.push(dataTypes.map((d) => d.toString()));
  }

  let timestampClause = "";
  if (timestamp) {
    timestampClause = `AND s.timestamp >= $${paramIdx++}::timestamptz - interval '${SCORE_TO_TRACE_OBSERVATIONS_INTERVAL} seconds'`;
    params.push(toPgTimestamp(timestamp));
  }

  let limitClause = "";
  if (limit !== undefined && offset !== undefined) {
    limitClause = `LIMIT $${paramIdx++} OFFSET $${paramIdx++}`;
    params.push(limit, offset);
  }

  const query = `
    SELECT * FROM (
      SELECT DISTINCT ON (s.id, s.project_id)
        ${select}
      FROM scores s
      WHERE s.project_id = $1
      AND s.trace_id = ANY($2::text[])
      ${dataTypeClause}
      ${timestampClause}
      ${levelFilter}
      ORDER BY s.id, s.project_id, s.updated_at DESC
    ) sub
    ${limitClause}
  `;

  const rows = await queryPg<
    ScoreRecordReadType & {
      metadata: ExcludeMetadata extends true
        ? never
        : ScoreRecordReadType["metadata"];
      has_metadata: IncludeHasMetadata extends true ? 0 | 1 : never;
    }
  >({
    query,
    params,
    tags: {
      feature: "tracing",
      type: "score",
      kind: "list",
      projectId,
    },
  });

  const includeMetadataPayload = excludeMetadata ? false : true;
  return rows.map((row) => {
    const score = convertClickhouseScoreToDomain(
      {
        ...row,
        metadata: excludeMetadata ? {} : row.metadata,
      },
      includeMetadataPayload,
    );

    recordDistribution(
      "langfuse.query_by_id_age",
      new Date().getTime() - score.timestamp.getTime(),
      {
        table: "scores",
      },
    );

    if (includeHasMetadata) {
      Object.assign(score, { hasMetadata: !!row.has_metadata });
    }

    return score;
  });
};

// Used in multiple places, including the public API, hence the non-default exclusion of metadata via excludeMetadata flag
export const getScoresForTraces = async <
  ExcludeMetadata extends boolean,
  IncludeHasMetadata extends boolean,
>(
  props: GetScoresForTracesProps<ExcludeMetadata, IncludeHasMetadata>,
) => {
  return getScoresForTracesInternal({
    ...props,
    dataTypes: LISTABLE_SCORE_TYPES,
  });
};

export const getScoresAndCorrectionsForTraces = async <
  ExcludeMetadata extends boolean,
  IncludeHasMetadata extends boolean,
>(
  props: GetScoresForTracesProps<ExcludeMetadata, IncludeHasMetadata>,
) => {
  return getScoresForTracesInternal({
    ...props,
  });
};

export type GetScoresForObservationsProps<
  ExcludeMetadata extends boolean,
  IncludeHasMetadata extends boolean,
> = {
  projectId: string;
  observationIds: string[];
  /**
   * When provided, adds `AND s.timestamp >= minTimestamp - SCORE_TO_TRACE_OBSERVATIONS_INTERVAL`
   * to the query so PG can prune partitions and avoid full-table scans.
   * Pass the minimum startTime of the observations whose scores you are fetching.
   */
  minTimestamp?: Date;
  limit?: number;
  offset?: number;
  excludeMetadata?: ExcludeMetadata;
  includeHasMetadata?: IncludeHasMetadata;
};

// Currently only used from the observations table, hence the exclusion of metadata without excludeMetadata flag
export const getScoresForObservations = async <
  ExcludeMetadata extends boolean,
  IncludeHasMetadata extends boolean,
>(
  props: GetScoresForObservationsProps<ExcludeMetadata, IncludeHasMetadata>,
) => {
  const {
    projectId,
    observationIds,
    minTimestamp,
    limit,
    offset,
    excludeMetadata = false,
    includeHasMetadata = false,
  } = props;

  const select = [
    !excludeMetadata ? "*" : [
      // PG-only: only columns that exist in the PG scores table
      "s.id", "s.timestamp", "s.project_id",
      "s.trace_id", "s.observation_id",
      "s.name", "s.value", "s.source", "s.comment", "s.author_user_id",
      "s.config_id", "s.data_type", "s.string_value",
      "s.queue_id", "s.created_at", "s.updated_at",
      // PG-mapped: columns that don't exist in PG → return defaults
      "'default' as environment",
      "NULL::text as session_id",
      "NULL::text as dataset_run_id",
      "NULL::text as long_string_value",
      "NULL::text as execution_trace_id",
      "s.updated_at as event_ts",
      "0 as is_deleted",
    ].join(", "),
    includeHasMetadata
      ? "0 as has_metadata"
      : null,
  ]
    .filter((s) => s != null)
    .join(", ");

  const params: unknown[] = [projectId, observationIds, LISTABLE_SCORE_TYPES];
  let paramIdx = 4;

  let tsClause = "";
  if (minTimestamp) {
    tsClause = `AND s.timestamp >= $${paramIdx++}::timestamptz - interval '${SCORE_TO_TRACE_OBSERVATIONS_INTERVAL} seconds'`;
    params.push(toPgTimestamp(minTimestamp));
  }

  let limitClause = "";
  if (limit !== undefined && offset !== undefined) {
    limitClause = `LIMIT $${paramIdx++} OFFSET $${paramIdx++}`;
    params.push(limit, offset);
  }

  const query = `
    SELECT * FROM (
      SELECT DISTINCT ON (s.id, s.project_id)
        ${select}
      FROM scores s
      WHERE s.project_id = $1
      AND s.observation_id = ANY($2::text[])
      AND s.data_type::text = ANY($3::text[])
      ${tsClause}
      ORDER BY s.id, s.project_id, s.updated_at DESC
    ) sub
    ${limitClause}
  `;

  const rows = await queryPg<
    ScoreRecordReadType & {
      metadata: ExcludeMetadata extends true
        ? never
        : ScoreRecordReadType["metadata"];
      has_metadata: IncludeHasMetadata extends true ? 0 | 1 : never;
    }
  >({
    query,
    params,
    tags: {
      feature: "tracing",
      type: "score",
      kind: "list",
      projectId,
    },
  });

  const includeMetadataPayload = excludeMetadata ? false : true;
  return rows.map((row) => ({
    ...convertClickhouseScoreToDomain(
      {
        ...row,
        metadata: excludeMetadata ? {} : row.metadata,
      },
      includeMetadataPayload,
    ),
    hasMetadata: (includeHasMetadata
      ? !!row.has_metadata
      : undefined) as IncludeHasMetadata extends true ? boolean : never,
  }));
};

/**
 * Event/experiment column mappings for building WHERE filters inside the events CTE.
 */
const scoresEventsFilterMapping = [
  {
    uiTableName: "Experiment IDs",
    uiTableId: "experimentIds",
    clickhouseTable: "events",
    clickhouseSelect: "experiment_id",
    queryPrefix: "e",
  },
];

export const getScoresGroupedByNameSourceType = async ({
  projectId,
  filter,
  fromTimestamp,
  toTimestamp,
}: {
  projectId: string;
  filter: FilterCondition[];
  fromTimestamp?: Date;
  toTimestamp?: Date;
}) => {
  const scoresFilter = new PgFilterList();
  scoresFilter.push(
    ...createFilterFromFilterStatePg(
      filter,
      scoresColumnsTableUiColumnDefinitions as any,
      scoresTableCols,
    ),
  );

  // Separate scores-only filters from event/experiment filters
  const nonEventFilters = scoresFilter.filter(
    (f) => !f.clickhouseTable.startsWith("events_"),
  );

  const scoresFilterRes = nonEventFilters.apply();

  // Only join dataset run items if there is a dataset run items filter
  const performDatasetRunItemsJoin = scoresFilter.some(
    (f) => f.clickhouseTable === "dataset_run_items_rmt",
  );

  // Extract event-level filter entries from the frontend filter state
  const eventFilterState = filter.filter((filterEntry) =>
    scoresEventsFilterMapping.some((col) =>
      matchesUiColumnMapping(col, filterEntry.column),
    ),
  );

  let eventsCTE = "";
  let eventsCTEParams: unknown[] = [];
  const hasEventsFilters = eventFilterState.length > 0;

  let eventsFilterRes: PgFilterResult | undefined;
  if (hasEventsFilters) {
    const cteFilters = new PgFilterList(
      createFilterFromFilterStatePg(eventFilterState, scoresEventsFilterMapping),
    );
    eventsFilterRes = cteFilters.apply();
    eventsCTEParams = eventsFilterRes.params;

    if (eventsFilterRes.query) {
      const cteQuery = buildExperimentTraceIdsCte(projectId);
      // Add the experiment IDs filter to the CTE params
      eventsCTE = `WITH experiment_events AS (
  ${cteQuery.query}
  AND ${eventsFilterRes.query}
)`;
      eventsCTEParams = [...cteQuery.params, ...eventsFilterRes.params];
    } else {
      const cteQuery = buildExperimentTraceIdsCte(projectId);
      eventsCTE = `WITH experiment_events AS (
  ${cteQuery.query}
)`;
      eventsCTEParams = cteQuery.params;
    }
  }

  const params: unknown[] = [projectId, LISTABLE_SCORE_TYPES];
  let paramIdx = 3;

  let fromClause = "";
  if (fromTimestamp) {
    fromClause = `AND s.timestamp >= $${paramIdx++}::timestamptz`;
    params.push(toPgTimestamp(fromTimestamp));
  }

  let toClause = "";
  if (toTimestamp) {
    toClause = `AND s.timestamp <= $${paramIdx++}::timestamptz`;
    params.push(toPgTimestamp(toTimestamp));
  }

  const query = `
    ${eventsCTE}
    SELECT
      s.name as name,
      s.source as source,
      s.data_type as data_type
    FROM scores s
    ${performDatasetRunItemsJoin ? `JOIN dataset_run_items dri ON s.trace_id = dri.trace_id AND s.project_id = dri.project_id` : ""}
    ${hasEventsFilters ? `JOIN experiment_events e ON s.trace_id = e.trace_id AND s.project_id = e.project_id` : ""}
    WHERE s.project_id = $1
    ${scoresFilterRes?.query ? `AND ${scoresFilterRes.query}` : ""}
    ${fromClause}
    ${toClause}
    AND s.data_type::text = ANY($2::text[])
    GROUP BY name, source, data_type
    ORDER BY count(*) desc
    LIMIT ${FILTER_OPTION_SCORE_NAME_LIMIT};
  `;

  const allParams = [
    ...params,
    ...(scoresFilterRes ? scoresFilterRes.params : []),
    ...eventsCTEParams,
  ];

  const rows = await queryPg<{
    name: string;
    source: string;
    data_type: string;
  }>({
    query,
    params: allParams,
    tags: {
      feature: "tracing",
      type: "score",
      kind: "list",
      projectId,
    },
  });

  return rows.map((row) => ({
    name: row.name,
    source: row.source as ScoreSourceType,
    dataType: row.data_type as ListableScoreDataType,
  }));
};

export const getNumericScoresGroupedByName = async (
  projectId: string,
  filter?: FilterState,
) => {
  // Despite the historical name of some callers, this accepts any score-table
  // compatible filter. Trace tables use this to scope discovery to scores that
  // roll up into trace aggregates, not just direct trace-level scores.
  const chFilter = filter
    ? createFilterFromFilterStatePg(
        filter,
        scoresColumnsTableUiColumnDefinitions as any,
        scoresTableCols,
      )
    : undefined;

  const filterRes = chFilter ? new PgFilterList(chFilter).apply() : undefined;

  const query = `
    SELECT
      name as name
    FROM scores s
    WHERE s.project_id = $1
    AND s.data_type IN ('NUMERIC', 'BOOLEAN')
    ${filterRes?.query ? `AND ${filterRes.query.replace(/\$(\d+)/g, (_: string, n: string) => "$" + (parseInt(n) + 1))}` : ""}
    GROUP BY name
    ORDER BY count(*) desc
    LIMIT ${FILTER_OPTION_SCORE_NAME_LIMIT};
  `;

  const rows = await queryPg<{
    name: string;
  }>({
    query,
    params: [
      projectId,
      ...(filterRes ? filterRes.params : []),
    ],
    tags: {
      feature: "tracing",
      type: "score",
      kind: "list",
      projectId,
    },
  });

  return rows;
};

export const getCategoricalScoresGroupedByName = async (
  projectId: string,
  filter?: FilterState,
) => {
  // Mirrors `getNumericScoresGroupedByName`: callers can provide any score
  // scope filters, not just timestamp predicates.
  const chFilter = filter
    ? createFilterFromFilterStatePg(
        filter,
        scoresColumnsTableUiColumnDefinitions as any,
        scoresTableCols,
      )
    : undefined;

  const filterRes = chFilter ? new PgFilterList(chFilter).apply() : undefined;

  const query = `
    SELECT
      name AS label,
      (array_agg(string_value))[1:${FILTER_OPTION_CATEGORICAL_VALUE_LIMIT}] AS values
    FROM scores s
    WHERE s.project_id = $1
    AND s.data_type = 'CATEGORICAL'
    ${filterRes?.query ? `AND ${filterRes.query.replace(/\$(\d+)/g, (_: string, n: string) => "$" + (parseInt(n) + 1))}` : ""}
    GROUP BY name
    ORDER BY count(*) DESC
    LIMIT ${FILTER_OPTION_SCORE_NAME_LIMIT};
  `;

  const rows = await queryPg<{
    label: string;
    values: string[];
  }>({
    query,
    params: [
      projectId,
      ...(filterRes ? filterRes.params : []),
    ],
    tags: {
      feature: "tracing",
      type: "score",
      kind: "list",
      projectId,
    },
  });

  // Get score names from PG results to query score configs
  const scoreNames = rows.map((row) => row.label);

  // Query score_configs table for categorical configurations
  const scoreConfigs =
    scoreNames.length > 0
      ? await prisma.scoreConfig.findMany({
          where: {
            projectId: projectId,
            name: {
              in: scoreNames,
            },
            dataType: "CATEGORICAL",
            isArchived: false,
          },
          select: {
            name: true,
            categories: true,
          },
        })
      : [];

  // Create a map of score configs for easy lookup
  const configMap = new Map(
    scoreConfigs.map((config) => [config.name, config.categories]),
  );

  // Enhance the results with all possible category values from score configs
  return rows.map((row) => {
    const configCategories = configMap.get(row.label);

    if (configCategories && Array.isArray(configCategories)) {
      // Extract all possible category labels from the score config
      const allPossibleValues = (
        configCategories as Array<{ label: string; value: number }>
      ).map((category) => category.label);

      // Merge actual values from PG with all possible values from config
      // Use Set to ensure uniqueness
      const mergedValues = Array.from(
        new Set([...row.values, ...allPossibleValues]),
      ).slice(0, FILTER_OPTION_CATEGORICAL_VALUE_LIMIT);

      return {
        ...row,
        values: mergedValues,
      };
    }

    // If no config found, return original values
    return row;
  });
};

export const getScoresUiCount = async (props: {
  projectId: string;
  filter: FilterState;
  orderBy: OrderByState;
  limit?: number;
  offset?: number;
}) => {
  const rows = await getScoresUiGeneric<{ count: string }>({
    select: "count",
    excludeMetadata: true,
    tags: { kind: "count" },
    ...props,
  });

  return Number(rows[0].count);
};

export type ScoreUiTableRow = ScoreDomain & {
  traceName: string | null;
  traceUserId: string | null;
  traceTags: Array<string> | null;
};

export async function getScoresUiTable<
  ExcludeMetadata extends boolean,
  IncludeHasMetadata extends boolean,
>(props: {
  projectId: string;
  filter: FilterState;
  orderBy: OrderByState;
  limit?: number;
  offset?: number;
  excludeMetadata?: ExcludeMetadata;
  includeHasMetadataFlag?: IncludeHasMetadata;
}) {
  const {
    excludeMetadata = false,
    includeHasMetadataFlag = false,
    ...rest
  } = props;

  const rows = await getScoresUiGeneric<{
    id: string;
    project_id: string;
    environment: string;
    name: string;
    value: number;
    string_value: string | null;
    timestamp: string;
    source: string;
    data_type: string;
    comment: string | null;
    trace_id: string | null;
    session_id: string | null;
    dataset_run_id: string | null;
    metadata: ExcludeMetadata extends true ? never : Record<string, string>;
    observation_id: string | null;
    author_user_id: string | null;
    user_id: string | null;
    trace_name: string | null;
    trace_tags: Array<string> | null;
    job_configuration_id: string | null;
    author_user_image: string | null;
    author_user_name: string | null;
    config_id: string | null;
    queue_id: string | null;
    execution_trace_id: string | null;
    is_deleted: number;
    event_ts: string;
    created_at: string;
    updated_at: string;
    has_metadata: IncludeHasMetadata extends true ? 0 | 1 : never;
  }>({
    select: "rows",
    tags: { kind: "analytic" },
    excludeMetadata,
    includeHasMetadataFlag,
    ...rest,
  });

  const includeMetadataPayload = excludeMetadata ? false : true;
  return rows.map((row) => {
    const score = convertClickhouseScoreToDomain(
      {
        ...row,
        metadata: excludeMetadata ? {} : row.metadata,
        long_string_value: "",
      },
      includeMetadataPayload,
    );
    return {
      ...score,
      traceUserId: row.user_id,
      traceName: row.trace_name,
      traceTags: row.trace_tags,
      hasMetadata: (includeHasMetadataFlag
        ? !!row.has_metadata
        : undefined) as IncludeHasMetadata extends true ? boolean : never,
    };
  });
}

const getScoresUiGeneric = async <T>(props: {
  select: "count" | "rows";
  projectId: string;
  filter: FilterState;
  orderBy: OrderByState;
  limit?: number;
  offset?: number;
  tags?: Record<string, string>;
  excludeMetadata?: boolean;
  includeHasMetadataFlag?: boolean;
}): Promise<T[]> => {
  const {
    projectId,
    filter,
    orderBy,
    limit,
    offset,
    excludeMetadata = false,
    includeHasMetadataFlag = false,
  } = props;

  const select =
    props.select === "count"
      ? "count(*) as count"
      : `
        s.id,
        s.project_id,
        s.environment,
        s.name,
        s.value,
        s.string_value,
        s.timestamp,
        s.source,
        s.data_type,
        s.comment,
        ${excludeMetadata ? "" : "s.metadata,"}
        s.trace_id,
        s.session_id,
        s.dataset_run_id,
        s.observation_id,
        s.author_user_id,
        t.user_id,
        t.name,
        t.tags,
        s.created_at,
        s.updated_at,
        s.source,
        s.config_id,
        s.queue_id,
        s.execution_trace_id,
        s.is_deleted,
        s.event_ts,
        t.user_id,
        t.name as trace_name,
        t.tags as trace_tags
        ${includeHasMetadataFlag ? ",CASE WHEN s.metadata IS NOT NULL AND s.metadata::text <> '{}'::text THEN 1 ELSE 0 END AS has_metadata" : ""}
      `;

  const { scoresFilter } = getProjectIdDefaultFilter(projectId, {
    tracesPrefix: "t",
  });
  scoresFilter.push(
    ...createFilterFromFilterStatePg(
      filter,
      scoresTableUiColumnDefinitions as any,
      scoresTableCols,
    ),
  );
  const scoresFilterRes = scoresFilter.apply();

  // Only join traces for rows or if there is a trace filter on counts
  const performTracesJoin =
    props.select === "rows" ||
    scoresFilter.some((f) => f.clickhouseTable === "traces");

  const orderByClause = orderByToPgSql(orderBy ?? null, "s");

  const params: unknown[] = [projectId, LISTABLE_SCORE_TYPES];
  let paramIdx = 3;

  let limitClause = "";
  if (limit !== undefined && offset !== undefined) {
    limitClause = `LIMIT $${paramIdx++} OFFSET $${paramIdx++}`;
    params.push(limit, offset);
  }

  const query = `
    SELECT
        ${select}
    FROM scores s
    ${performTracesJoin ? "LEFT JOIN traces t ON s.trace_id = t.id AND t.project_id = s.project_id" : ""}
    WHERE s.project_id = $1
    AND s.data_type::text = ANY($2::text[])
    ${scoresFilterRes?.query ? `AND ${scoresFilterRes.query}` : ""}
    ${orderByClause}
    ${limitClause}
  `;

  const allParams = [
    ...params,
    ...(scoresFilterRes ? scoresFilterRes.params : []),
  ];

  return measureAndReturn({
    operationName: "getScoresUiGeneric",
    projectId,
    input: {
      params: allParams,
      tags: {
        ...(props.tags ?? {}),
        feature: "tracing",
        type: "score",
        projectId,
        select: props.select,
        operation_name: "getScoresUiGeneric",
      },
    },
    fn: async (input) => {
      return queryPg<T>({
        query,
        params: input.params,
        tags: input.tags,
      });
    },
  });
};

/**
 * Trace column mapping for building WHERE filters inside the flat events CTE.
 */
const scoresTraceFilterEventsMapping = [
  {
    uiTableName: "Trace Name",
    uiTableId: "traceName",
    clickhouseTable: "traces",
    clickhouseSelect: "trace_name",
    queryPrefix: "e",
  },
  {
    uiTableName: "User ID",
    uiTableId: "userId",
    clickhouseTable: "traces",
    clickhouseSelect: "user_id",
    queryPrefix: "e",
  },
  {
    uiTableName: "Trace Tags",
    uiTableId: "trace_tags",
    clickhouseTable: "traces",
    clickhouseSelect: "tags",
    queryPrefix: "e",
  },
];

/**
 * v4 variant: scores query using a flat events CTE instead of the physical
 * traces table. Trace-level filters and sort use a "traces" CTE built from
 * events.
 */
const getScoresUiGenericFromEvents = async <T>(props: {
  select: "count" | "rows";
  projectId: string;
  filter: FilterState;
  orderBy: OrderByState;
  limit?: number;
  offset?: number;
  tags?: Record<string, string>;
  excludeMetadata?: boolean;
  includeHasMetadataFlag?: boolean;
}): Promise<T[]> => {
  const {
    projectId,
    filter,
    orderBy,
    limit,
    offset,
    excludeMetadata = false,
    includeHasMetadataFlag = false,
  } = props;

  const { scoresFilter } = getProjectIdDefaultFilter(projectId, {
    tracesPrefix: "t",
  });
  scoresFilter.push(
    ...createFilterFromFilterStatePg(
      filter,
      scoresTableUiColumnDefinitionsFromEvents as any,
      scoresTableCols,
    ),
  );

  const scoreOnlyFilters = scoresFilter.filter(
    (f) => f.clickhouseTable !== "traces",
  );
  const scoreOnlyFilterRes = scoreOnlyFilters.apply();

  // Trace-level filter entries from the frontend filter state
  const traceFilterState = filter.filter((filterEntry) =>
    scoresTraceFilterEventsMapping.some((col) =>
      matchesUiColumnMapping(col, filterEntry.column),
    ),
  );

  const matchedOrderByColumn = orderBy
    ? findUiColumnMapping(
        scoresTableUiColumnDefinitionsFromEvents,
        orderBy.column,
      )
    : null;
  const orderByColumn =
    matchedOrderByColumn?.clickhouseTableName === "traces"
      ? matchedOrderByColumn
      : null;

  const needsTracesCTE = traceFilterState.length > 0 || !!orderByColumn;

  // Build traces CTE using events table when needed
  let tracesCTEClause = "";
  const tracesCTEParams: unknown[] = [];

  if (needsTracesCTE) {
    const cteQuery = buildTracesMetadataCte(projectId);

    if (traceFilterState.length > 0) {
      const cteTraceFilters = new PgFilterList(
        createFilterFromFilterStatePg(
          traceFilterState,
          scoresTraceFilterEventsMapping,
          scoresTableCols,
        ),
      );
      const cteTraceFilterRes = cteTraceFilters.apply();
      if (cteTraceFilterRes.query) {
        tracesCTEClause = `WITH traces AS (
  ${cteQuery.query}
  AND ${cteTraceFilterRes.query}
)`;
        tracesCTEParams.push(...cteQuery.params, ...cteTraceFilterRes.params);
      }
    } else {
      tracesCTEClause = `WITH traces AS (
  ${cteQuery.query}
)`;
      tracesCTEParams.push(...cteQuery.params);
    }
  }

  // Inner join when trace filters are active (exclude scores without matching traces)
  // Left join when only sorting (keep all scores)
  const eventsJoin = needsTracesCTE
    ? traceFilterState.length > 0
      ? `JOIN traces e ON s.trace_id = e.id`
      : `LEFT JOIN traces e ON s.trace_id = e.id`
    : "";

  const select =
    props.select === "count"
      ? "count(*) as count"
      : `
        s.id,
        s.project_id,
        s.environment,
        s.name,
        s.value,
        s.string_value,
        s.timestamp,
        s.source,
        s.data_type,
        s.comment,
        ${excludeMetadata ? "" : "s.metadata,"}
        s.trace_id,
        s.session_id,
        s.dataset_run_id,
        s.observation_id,
        s.author_user_id,
        s.created_at,
        s.updated_at,
        s.config_id,
        s.queue_id,
        s.execution_trace_id,
        s.is_deleted,
        s.event_ts
        ${includeHasMetadataFlag ? ",CASE WHEN s.metadata IS NOT NULL AND s.metadata::text <> '{}'::text THEN 1 ELSE 0 END AS has_metadata" : ""}
      `;

  const orderByClause = orderByToPgSql(orderBy ?? null, "s");

  const params: unknown[] = [projectId, LISTABLE_SCORE_TYPES];
  let paramIdx = 3;

  let limitClause = "";
  if (limit !== undefined && offset !== undefined) {
    limitClause = `LIMIT $${paramIdx++} OFFSET $${paramIdx++}`;
    params.push(limit, offset);
  }

  const query = `
    ${tracesCTEClause}
    SELECT
        ${select}
    FROM scores s
    ${eventsJoin}
    WHERE s.project_id = $1
    AND s.data_type::text = ANY($2::text[])
    ${scoreOnlyFilterRes?.query ? `AND ${scoreOnlyFilterRes.query}` : ""}
    ${orderByClause}
    ${limitClause}
  `;

  const allParams = [
    ...params,
    ...(scoreOnlyFilterRes ? scoreOnlyFilterRes.params : []),
    ...tracesCTEParams,
  ];

  return measureAndReturn({
    operationName: "getScoresUiGenericFromEvents",
    projectId,
    input: {
      params: allParams,
      tags: {
        ...(props.tags ?? {}),
        feature: "tracing",
        type: "score",
        projectId,
        select: props.select,
        operation_name: "getScoresUiGenericFromEvents",
      },
    },
    fn: async (input) => {
      return queryPg<T>({
        query,
        params: input.params,
        tags: input.tags,
      });
    },
  });
};

export const getScoresUiCountFromEvents = async (props: {
  projectId: string;
  filter: FilterState;
  orderBy: OrderByState;
  limit?: number;
  offset?: number;
}) => {
  const rows = await getScoresUiGenericFromEvents<{ count: string }>({
    select: "count",
    excludeMetadata: true,
    tags: { kind: "count" },
    ...props,
  });

  return Number(rows[0].count);
};

export type ScoreUiTableRowFromEvents = Omit<ScoreDomain, "metadata"> & {
  hasMetadata: boolean;
};

export async function getScoresUiTableFromEvents(props: {
  projectId: string;
  filter: FilterState;
  orderBy: OrderByState;
  limit?: number;
  offset?: number;
  // Defaults to true: the scores UI table only needs the hasMetadata flag.
  // Batch exports pass false to include the full metadata payload.
  excludeMetadata?: boolean;
}) {
  const { excludeMetadata = true, ...rest } = props;

  const rows = await getScoresUiGenericFromEvents<{
    id: string;
    project_id: string;
    environment: string;
    name: string;
    value: number;
    string_value: string | null;
    timestamp: string;
    source: string;
    data_type: string;
    comment: string | null;
    trace_id: string | null;
    session_id: string | null;
    dataset_run_id: string | null;
    metadata?: Record<string, string>;
    observation_id: string | null;
    author_user_id: string | null;
    config_id: string | null;
    queue_id: string | null;
    execution_trace_id: string | null;
    is_deleted: number;
    event_ts: string;
    created_at: string;
    updated_at: string;
    has_metadata: 0 | 1;
  }>({
    select: "rows",
    tags: { kind: "analytic" },
    excludeMetadata,
    includeHasMetadataFlag: true,
    ...rest,
  });

  return rows.map((row) => {
    const score = convertClickhouseScoreToDomain(
      {
        ...row,
        metadata: excludeMetadata ? {} : (row.metadata ?? {}),
        long_string_value: "",
      },
      !excludeMetadata,
    );
    return {
      ...score,
      hasMetadata: !!row.has_metadata,
    };
  });
}

export const getScoreNames = async (
  projectId: string,
  timestampFilter: FilterState,
) => {
  const chFilter = new PgFilterList(
    createFilterFromFilterStatePg(
      timestampFilter,
      scoresTableUiColumnDefinitions as any,
      scoresTableCols,
    ),
  );
  const timestampFilterRes = chFilter.apply();

  const query = `
    SELECT
      name,
      count(*) as count
    FROM scores s
    WHERE s.project_id = $1
    ${timestampFilterRes?.query ? `AND ${timestampFilterRes.query}` : ""}
    AND s.data_type::text = ANY($2::text[])
    GROUP BY name
    ORDER BY count(*) desc
    LIMIT 1000;
  `;

  const rows = await queryPg<{
    name: string;
    count: string;
  }>({
    query,
    params: [
      projectId,
      LISTABLE_SCORE_TYPES,
      ...(timestampFilterRes ? timestampFilterRes.params : []),
    ],
    tags: {
      feature: "tracing",
      type: "score",
      kind: "list",
      projectId,
    },
  });

  return rows.map((row) => ({
    name: row.name,
    count: Number(row.count),
  }));
};

export const getScoreStringValues = async (
  projectId: string,
  timestampFilter: FilterState,
) => {
  const chFilter = new PgFilterList(
    createFilterFromFilterStatePg(
      timestampFilter,
      scoresTableUiColumnDefinitions as any,
      scoresTableCols,
    ),
  );
  const timestampFilterRes = chFilter.apply();

  // exclude TEXT scores as they are arbitrary by nature and hence have high cardinality
  // which in turn can lead to performance issues
  const query = `
    SELECT
      string_value,
      count(*) as count
    FROM scores s
    WHERE s.project_id = $1
    AND string_value IS NOT NULL
    AND string_value != ''
    AND s.data_type != 'TEXT'
    ${timestampFilterRes?.query ? `AND ${timestampFilterRes.query}` : ""}
    GROUP BY string_value
    ORDER BY count(*) desc
    LIMIT 1000;
  `;

  const rows = await queryPg<{
    string_value: string;
    count: string;
  }>({
    query,
    params: [
      projectId,
      ...(timestampFilterRes ? timestampFilterRes.params : []),
    ],
    tags: {
      feature: "tracing",
      type: "score",
      kind: "list",
      projectId,
    },
  });

  return rows.map((row) => ({
    value: row.string_value,
    count: Number(row.count),
  }));
};

export const deleteScores = async (projectId: string, scoreIds: string[]) => {
  const query = `
    DELETE FROM scores
    WHERE project_id = $1
    AND id = ANY($2::text[]);
  `;
  await executePg({
    query,
    params: [projectId, scoreIds],
    tags: {
      feature: "tracing",
      type: "score",
      kind: "delete",
      projectId,
    },
  });
};

export const deleteScoresByTraceIds = async (
  projectId: string,
  traceIds: string[],
) => {
  const query = `
    DELETE FROM scores
    WHERE project_id = $1
    AND trace_id = ANY($2::text[]);
  `;
  await executePg({
    query,
    params: [projectId, traceIds],
    tags: {
      feature: "tracing",
      type: "score",
      kind: "delete",
      projectId,
    },
  });
};

export const deleteScoresByProjectId = async (
  projectId: string,
): Promise<boolean> => {
  const hasData = await hasAnyScore(projectId);
  if (!hasData) {
    return false;
  }

  const query = `
    DELETE FROM scores
    WHERE project_id = $1;
  `;
  await executePg({
    query,
    params: [projectId],
    tags: {
      feature: "tracing",
      type: "score",
      kind: "delete",
      projectId,
    },
  });

  return true;
};

export const hasAnyScoreOlderThan = async (
  projectId: string,
  beforeDate: Date,
) => {
  const query = `
    SELECT 1
    FROM scores
    WHERE project_id = $1
    AND timestamp < $2::timestamptz
    LIMIT 1
  `;

  const rows = await queryPg<{ "?column?": number }>({
    query,
    params: [projectId, toPgTimestamp(beforeDate)],
    tags: {
      feature: "tracing",
      type: "score",
      kind: "hasAnyOlderThan",
      projectId,
    },
  });

  return rows.length > 0;
};

export const deleteScoresOlderThanDays = async (
  projectId: string,
  beforeDate: Date,
): Promise<boolean> => {
  const hasData = await hasAnyScoreOlderThan(projectId, beforeDate);
  if (!hasData) {
    return false;
  }

  const query = `
    DELETE FROM scores
    WHERE project_id = $1
    AND timestamp < $2::timestamptz;
  `;
  await executePg({
    query,
    params: [projectId, toPgTimestamp(beforeDate)],
    tags: {
      feature: "tracing",
      type: "score",
      kind: "delete",
      projectId,
    },
  });

  return true;
};

export const getNumericScoreHistogram = async (
  projectId: string,
  filter: FilterState,
  limit: number,
) => {
  const chFilter = new PgFilterList(
    createFilterFromFilterStatePg(filter, dashboardColumnDefinitions as any),
  );
  const chFilterRes = chFilter.apply();

  const traceFilter = chFilter.find((f) => f.clickhouseTable === "traces");

  const params: unknown[] = [projectId];
  let paramIdx = 2;

  let limitClause = "";
  if (limit !== undefined) {
    limitClause = `LIMIT $${paramIdx++}`;
    params.push(limit);
  }

  const query = `
    SELECT * FROM (
      SELECT DISTINCT ON (s.id, s.project_id)
        s.value
      FROM scores s
      ${traceFilter ? `LEFT JOIN traces t ON s.trace_id = t.id AND t.project_id = s.project_id` : ""}
      WHERE s.project_id = $1
      ${traceFilter ? `AND t.project_id = $1` : ""}
      ${chFilterRes?.query ? `AND ${chFilterRes.query}` : ""}
      ORDER BY s.id, s.project_id, s.updated_at DESC
    ) sub
    ${limitClause}
  `;

  return measureAndReturn({
    operationName: "getNumericScoreHistogram",
    projectId,
    input: {
      params: [
        ...params,
        ...(chFilterRes ? chFilterRes.params : []),
      ],
      tags: {
        feature: "tracing",
        type: "score",
        kind: "analytic",
        projectId,
        operation_name: "getNumericScoreHistogram",
      },
    },
    fn: async (input) => {
      return queryPg<{ value: number }>({
        query,
        params: input.params,
        tags: input.tags,
      });
    },
  });
};

export const getAggregatedScoresForPrompts = async (
  projectId: string,
  promptIds: string[],
  fetchScoreRelation: "observation" | "trace",
  {
    fromTimestamp,
    toTimestamp,
  }: { fromTimestamp?: Date; toTimestamp?: Date } = {},
) => {
  const params: unknown[] = [projectId, projectId, promptIds];
  let paramIdx = 4;

  let fromClause = "";
  if (fromTimestamp) {
    fromClause = `AND o.start_time >= $${paramIdx++}::timestamptz`;
    params.push(toPgTimestamp(fromTimestamp));
  }

  let toClause = "";
  if (toTimestamp) {
    toClause = `AND o.start_time <= $${paramIdx++}::timestamptz`;
    params.push(toPgTimestamp(toTimestamp));
  }

  const query = `
    SELECT
      o.prompt_id as prompt_id,
      s.id,
      s.name,
      s.string_value,
      s.value,
      s.source,
      s.data_type,
      s.comment,
      s.timestamp,
      CASE WHEN s.metadata IS NOT NULL AND s.metadata::text <> '{}'::text THEN 1 ELSE 0 END AS has_metadata
    FROM scores s
    JOIN observations o
      ON o.trace_id = s.trace_id
      AND o.project_id = s.project_id
      ${fetchScoreRelation === "observation" ? "AND o.id = s.observation_id" : ""}
    WHERE o.project_id = $1
    AND s.project_id = $2
    AND o.prompt_id = ANY($3::text[])
    AND o.type = 'GENERATION'
    ${fromClause}
    ${toClause}
    AND s.name IS NOT NULL
    ${fetchScoreRelation === "trace" ? "AND s.observation_id IS NULL" : ""}
    AND s.data_type::text = ANY($${paramIdx++}::text[])
  `;
  params.push(LISTABLE_SCORE_TYPES);

  const rows = await queryPg<
    ScoreAggregation & {
      prompt_id: string;
      has_metadata: 0 | 1;
    }
  >({
    query,
    params,
    tags: {
      feature: "tracing",
      type: "score",
      kind: "analytic",
      projectId,
    },
  });

  return rows.map((row) => ({
    ...convertScoreAggregation<ListableScoreDataType>(row),
    promptId: row.prompt_id,
    hasMetadata: !!row.has_metadata,
  }));
};

export const getScoreCountsByProjectInCreationInterval = async ({
  start,
  end,
}: {
  start: Date;
  end: Date;
}) => {
  const query = `
    SELECT
      project_id,
      count(*) as count
    FROM scores
    WHERE created_at >= $1::timestamptz
    AND created_at < $2::timestamptz
    AND data_type::text = ANY($3::text[])
    GROUP BY project_id
  `;

  const rows = await queryPg<{ project_id: string; count: string }>({
    query,
    params: [
      toPgTimestamp(start),
      toPgTimestamp(end),
      LISTABLE_SCORE_TYPES,
    ],
    tags: {
      feature: "tracing",
      type: "score",
      kind: "analytic",
    },
  });

  return rows.map((row) => ({
    projectId: row.project_id,
    count: Number(row.count),
  }));
};

export const getScoreCountOfProjectsSinceCreationDate = async ({
  projectIds,
  start,
}: {
  projectIds: string[];
  start: Date;
}) => {
  const query = `
    SELECT
      count(*) as count
    FROM scores
    WHERE project_id = ANY($1::text[])
    AND created_at >= $2::timestamptz
  `;

  const rows = await queryPg<{ count: string }>({
    query,
    params: [projectIds, toPgTimestamp(start)],
    tags: {
      feature: "tracing",
      type: "score",
      kind: "analytic",
    },
  });

  return Number(rows[0]?.count ?? 0);
};

export const getDistinctScoreNames = async (p: {
  projectId: string;
  cutoffCreatedAt: Date;
  filter: FilterState;
  isTimestampFilter: (filter: FilterCondition) => filter is TimeFilter;
}) => {
  const {
    projectId,
    cutoffCreatedAt,
    filter,
    isTimestampFilter,
  } = p;
  const scoreTimestampFilter = filter?.find(isTimestampFilter);

  const params: unknown[] = [projectId, toPgTimestamp(cutoffCreatedAt), LISTABLE_SCORE_TYPES];
  let paramIdx = 4;

  let tsClause = "";
  if (scoreTimestampFilter) {
    tsClause = `AND s.timestamp >= $${paramIdx++}::timestamptz`;
    params.push(toPgTimestamp(scoreTimestampFilter.value));
  }

  const query = `
    SELECT DISTINCT
      name
    FROM scores s
    WHERE s.project_id = $1
    AND s.created_at <= $2::timestamptz
    ${tsClause}
    AND s.data_type::text = ANY($3::text[])
  `;

  const rows = await queryPg<{ name: string }>({
    query,
    params,
    tags: {
      feature: "tracing",
      type: "score",
      kind: "list",
      projectId,
    },
  });

  return rows.map((row) => row.name);
};

export const getScoresForBlobStorageExport = function (
  projectId: string,
  minTimestamp: Date,
  maxTimestamp: Date,
) {
  const query = `
    SELECT
      id,
      timestamp,
      project_id,
      environment,
      trace_id,
      observation_id,
      session_id,
      dataset_run_id,
      name,
      value,
      source,
      comment,
      data_type,
      string_value,
      created_at,
      updated_at
    FROM scores
    WHERE project_id = $1
    AND timestamp >= $2::timestamptz
    AND timestamp <= $3::timestamptz
    AND data_type::text = ANY($4::text[])
  `;

  const records = queryPgStream<Record<string, unknown>>({
    query,
    params: [
      projectId,
      toPgTimestamp(minTimestamp),
      toPgTimestamp(maxTimestamp),
      LISTABLE_SCORE_TYPES,
    ],
    tags: {
      feature: "blobstorage",
      type: "score",
      kind: "analytic",
      projectId,
    },
  });

  return records;
};

export const getScoresForAnalyticsIntegrations = async function* (
  projectId: string,
  projectName: string,
  minTimestamp: Date,
  maxTimestamp: Date,
  options: { useGraceHash?: boolean } = {},
) {
  // Pre-filter traces in a CTE so the trace timestamp window prunes partitions
  // directly, instead of living in an OR clause after the LEFT JOIN where the
  // planner cannot push it down. Subtract 7d from minTimestamp to keep scores
  // whose trace was created before the score window started.
  const cteMinTs = toPgTimestamp(
    new Date(minTimestamp.getTime() - 7 * 24 * 60 * 60 * 1000),
  );

  const query = `
    WITH selected_traces AS (
      SELECT
        t.project_id as project_id,
        t.id as id,
        t.name as name,
        t.session_id as session_id,
        t.user_id as user_id,
        t.release as release,
        t.tags as tags,
        t.metadata->>'$posthog_session_id' as posthog_session_id,
        t.metadata->>'$mixpanel_session_id' as mixpanel_session_id
      FROM traces t
      WHERE t.project_id = $1
      AND t.timestamp >= $2::timestamptz
      AND t.timestamp <= $3::timestamptz
    )

    SELECT
      s.id as id,
      s.timestamp as timestamp,
      s.name as name,
      s.value as value,
      s.string_value as string_value,
      s.data_type as data_type,
      s.comment as comment,
      s.environment as environment,
      s.trace_id as score_trace_id,
      s.session_id as score_session_id,
      s.dataset_run_id as score_dataset_run_id,
      t.id as trace_id,
      t.name as trace_name,
      t.session_id as trace_session_id,
      t.user_id as trace_user_id,
      t.release as trace_release,
      t.tags as trace_tags,
      s.metadata as metadata,
      t.posthog_session_id as posthog_session_id,
      t.mixpanel_session_id as mixpanel_session_id
    FROM scores s
    LEFT JOIN selected_traces t ON s.trace_id = t.id AND s.project_id = t.project_id
    WHERE s.project_id = $1
    AND s.timestamp >= $4::timestamptz
    AND s.timestamp < $5::timestamptz
    AND s.data_type::text = ANY($6::text[])
    AND (
      s.trace_id IS NOT NULL
      OR s.session_id IS NOT NULL
      OR s.dataset_run_id IS NOT NULL
    )
  `;

  const records = queryPgStream<Record<string, unknown>>({
    query,
    params: [
      projectId,
      cteMinTs,
      toPgTimestamp(maxTimestamp),
      toPgTimestamp(minTimestamp),
      toPgTimestamp(maxTimestamp),
      LISTABLE_SCORE_TYPES,
    ],
    tags: {
      feature: "posthog",
      type: "score",
      kind: "analytic",
      projectId,
    },
  });

  const baseUrl = env.NEXTAUTH_URL?.replace("/api/auth", "");
  for await (const record of records) {
    // Determine the effective session_id based on score attachment
    const effectiveSessionId =
      record.score_session_id || record.trace_session_id;

    // Determine the effective trace_id (could be null for session-only or dataset-run-only scores)
    const effectiveTraceId = record.score_trace_id || null;

    yield {
      timestamp: record.timestamp,
      langfuse_score_name: record.name,
      langfuse_score_value: record.value,
      langfuse_score_comment: record.comment,
      langfuse_score_metadata: record.metadata,
      langfuse_score_string_value: record.string_value,
      langfuse_score_data_type: record.data_type,
      langfuse_trace_name: record.trace_name,
      langfuse_trace_id: effectiveTraceId,
      langfuse_user_url: record.trace_user_id
        ? `${baseUrl}/project/${projectId}/users/${encodeURIComponent(record.trace_user_id as string)}`
        : undefined,
      langfuse_id: record.id,
      langfuse_session_id: effectiveSessionId,
      langfuse_project_id: projectId,
      langfuse_project_name: projectName,
      langfuse_user_id: record.trace_user_id || null,
      langfuse_release: record.trace_release,
      langfuse_tags: record.trace_tags,
      langfuse_environment: record.environment,
      langfuse_event_version: "1.0.0",
      langfuse_score_entity_type: record.score_trace_id
        ? "trace"
        : record.score_session_id
          ? "session"
          : record.score_dataset_run_id
            ? "dataset_run"
            : "unknown",
      langfuse_dataset_run_id: record.score_dataset_run_id,
      posthog_session_id: record.posthog_session_id ?? null,
      mixpanel_session_id: record.mixpanel_session_id ?? null,
    } satisfies AnalyticsScoreEvent;
  }
};

export const hasAnyScore = async (projectId: string) => {
  const query = `
    SELECT 1
    FROM scores
    WHERE project_id = $1
    LIMIT 1
  `;

  const rows = await queryPg<{ "?column?": number }>({
    query,
    params: [projectId],
    tags: {
      feature: "tracing",
      type: "score",
      kind: "hasAny",
      projectId,
    },
  });

  return rows.length > 0;
};

export const getScoreMetadataById = async (
  projectId: string,
  id: string,
  source?: ScoreSourceType,
) => {
  const params: unknown[] = [projectId, id];
  let paramIdx = 3;

  let sourceClause = "";
  if (source) {
    sourceClause = `AND s.source = $${paramIdx++}`;
    params.push(source);
  }

  const query = `
    SELECT * FROM (
      SELECT DISTINCT ON (s.id, s.project_id)
        s.metadata
      FROM scores s
      WHERE s.project_id = $1
      AND s.id = $2
      ${sourceClause}
      ORDER BY s.id, s.project_id, s.updated_at DESC
    ) sub
    LIMIT 1
  `;

  const rows = await queryPg<Pick<ScoreRecordReadType, "metadata">>({
    query,
    params,
    tags: {
      feature: "tracing",
      type: "score",
      kind: "getScoreMetadataById",
      projectId,
    },
  });

  return rows
    .map((row) =>
      parseMetadataCHRecordToDomain(row.metadata as Record<string, string>),
    )
    .shift();
};

/**
 * Get score counts grouped by project and day within a date range.
 *
 * Returns one row per project per day with the count of scores created on that day.
 * Uses half-open interval [startDate, endDate) for filtering based on timestamp.
 *
 * @param startDate - Start of date range (inclusive)
 * @param endDate - End of date range (exclusive)
 * @returns Array of { count, projectId, date } objects
 *
 * @example
 * // Get score counts for March 1-2, 2024
 * const counts = await getScoreCountsByProjectAndDay({
 *   startDate: new Date('2024-03-01T00:00:00Z'),
 *   endDate: new Date('2024-03-03T00:00:00Z')
 * });
 *
 */
export const getScoreCountsByProjectAndDay = async ({
  startDate,
  endDate,
}: {
  startDate: Date;
  endDate: Date;
}) => {
  const query = `
    SELECT
      count(*) as count,
      project_id,
      timestamp::date as date
    FROM scores
    WHERE timestamp >= $1::timestamptz
    AND timestamp < $2::timestamptz
    AND data_type::text = ANY($3::text[])
    GROUP BY project_id, timestamp::date
  `;

  const rows = await queryPg<{
    count: string;
    project_id: string;
    date: string;
  }>({
    query,
    params: [
      toPgTimestamp(startDate),
      toPgTimestamp(endDate),
      LISTABLE_SCORE_TYPES,
    ],
    tags: {
      feature: "tracing",
      type: "score",
      kind: "analytic",
    },
  });

  return rows.map((row) => ({
    count: Number(row.count),
    projectId: row.project_id,
    date: row.date,
  }));
};

// ─── Cursor helpers (v3 pagination) ───────────────────────────────────────────

export const ScoresCursorV3 = z.discriminatedUnion("v", [
  z.object({
    v: z.literal(1),
    lastTimestamp: z.coerce.date(),
    lastId: z.string(),
  }),
]);
export type ScoresCursorV3Type = z.infer<typeof ScoresCursorV3>;

export const EncodedScoresCursorV3 = z
  .string()
  .transform((val) => {
    try {
      const decoded = Buffer.from(val, "base64url").toString("utf-8");
      return JSON.parse(decoded);
    } catch (_e) {
      throw new InvalidRequestError("Invalid cursor format");
    }
  })
  .pipe(ScoresCursorV3);

export const encodeCursorV3 = (cursor: ScoresCursorV3Type): string =>
  Buffer.from(
    JSON.stringify({
      v: cursor.v,
      lastTimestamp: cursor.lastTimestamp.toISOString(),
      lastId: cursor.lastId,
    }),
  ).toString("base64url");

// ─── v1/v2 public-API score query helpers ─────────────────────────────────────

export type ScoreQueryType = {
  page: number;
  limit: number;
  projectId: string;
  traceId?: string;
  userId?: string;
  name?: string;
  source?: string;
  fromTimestamp?: string;
  toTimestamp?: string;
  value?: number;
  scoreId?: string;
  configId?: string;
  sessionId?: string;
  datasetRunId?: string;
  queueId?: string;
  traceTags?: string | string[];
  operator?: string;
  scoreIds?: string[];
  observationId?: string[];
  dataType?: string;
  environment?: string | string[];
  fields?: string[] | null;
  advancedFilters?: FilterState;
};

export const _handleGenerateScoresForPublicApi = async ({
  projectId,
  scoresFilter,
  tracesFilter,
  scoreScope,
  includeTrace,
  needsTraceJoin,
  pagination,
  apiVersion,
}: {
  projectId: string;
  scoresFilter: PgFilterList;
  tracesFilter: PgFilterList;
  scoreScope: "traces_only" | "all";
  includeTrace: boolean;
  needsTraceJoin: boolean;
  pagination?: { limit: number; page: number };
  apiVersion?: "v1" | "v2";
}) => {
  const appliedScoresFilter = scoresFilter.apply();
  const appliedTracesFilter = tracesFilter.apply();

  const params: unknown[] = [
    projectId,
    ...appliedScoresFilter.params,
  ];
  const extraParams: unknown[] = [];
  let paramIdx = 1;

  let paginationClause = "";
  if (pagination !== undefined) {
    paginationClause = `LIMIT $${paramIdx++} OFFSET $${paramIdx++}`;
    extraParams.push(pagination.limit, (pagination.page - 1) * pagination.limit);
  }

  const query = `
      SELECT
          ${needsTraceJoin ? "t.user_id as user_id, t.tags as tags, t.environment as trace_environment, t.session_id as trace_session_id," : ""}
          s.id as id,
          s.project_id as project_id,
          s.timestamp as timestamp,
          s.environment as environment,
          s.name as name,
          s.value as value,
          s.string_value as string_value,
          s.long_string_value as long_string_value,
          s.author_user_id as author_user_id,
          s.created_at as created_at,
          s.updated_at as updated_at,
          s.source as source,
          s.comment as comment,
          s.metadata as metadata,
          s.data_type as data_type,
          s.config_id as config_id,
          s.queue_id as queue_id,
          s.execution_trace_id as execution_trace_id,
          s.trace_id as trace_id,
          s.observation_id as observation_id,
          s.session_id as session_id,
          s.dataset_run_id as dataset_run_id
      FROM
          scores s
          ${needsTraceJoin ? "LEFT JOIN traces t ON s.trace_id = t.id AND s.project_id = t.project_id" : ""}
      WHERE
          s.project_id = $1
          AND (
            ${scoreScope === "traces_only" ? "" : "s.trace_id IS NULL OR "}
            (s.trace_id IS NOT NULL AND (${needsTraceJoin ? "t.id" : "s.trace_id"}) IN (
              SELECT DISTINCT ON (inner_s.id, inner_s.project_id)
                ${needsTraceJoin ? "inner_s.trace_id" : "inner_s.trace_id"}
              FROM
                scores inner_s
              WHERE
                inner_s.project_id = $1
                ${appliedScoresFilter.query ? `AND ${appliedScoresFilter.query.replace(/\$(\d+)/g, (_m: string, n: string) => `$${parseInt(n)}`)}` : ""}
                ${scoreScope === "traces_only" ? "AND inner_s.session_id IS NULL AND inner_s.dataset_run_id IS NULL" : ""}
              ORDER BY
                inner_s.id, inner_s.project_id, inner_s.timestamp DESC
                ))
          )
          ${scoreScope === "traces_only" ? "AND s.session_id IS NULL AND s.dataset_run_id IS NULL" : ""}
          ${appliedScoresFilter.query ? `AND ${appliedScoresFilter.query}` : ""}
          ${tracesFilter.length() > 0 ? `AND ${appliedTracesFilter.query}` : ""}
      ORDER BY
          s.timestamp DESC, s.updated_at DESC
      ${paginationClause}
      `;

  return measureAndReturn({
    operationName: "_handleGenerateScoresForPublicApi",
    projectId,
    input: {
      params: [
        ...params,
        ...appliedTracesFilter.params,
        ...extraParams,
      ],
      tags: {
        feature: "scoring",
        type: "score",
        projectId,
        scoreScope,
        operation_name: "_handleGenerateScoresForPublicApi",
        includeTrace: includeTrace.toString(),
        ...(apiVersion ? { api_version: apiVersion } : {}),
      },
    },
    fn: async (input) => {
      const records = await queryPg<
        ScoreRecordReadType & {
          tags?: string[];
          user_id?: string;
          trace_environment?: string;
          trace_session_id?: string | null;
        }
      >({
        query,
        params: input.params,
        tags: input.tags,
      });

      return records.map((record) => {
        const domainScore = convertClickhouseScoreToDomain(record);
        return {
          ...domainScore,
          trace:
            includeTrace && record.trace_id !== null
              ? {
                  userId: record.user_id,
                  tags: record.tags,
                  environment: record.trace_environment,
                  sessionId: record.trace_session_id,
                }
              : null,
        };
      });
    },
  });
};

export const _handleGetScoresCountForPublicApi = async ({
  projectId,
  scoresFilter,
  tracesFilter,
  scoreScope,
  includeTrace,
  needsTraceJoin,
  apiVersion,
}: {
  projectId: string;
  scoresFilter: PgFilterList;
  tracesFilter: PgFilterList;
  scoreScope: "traces_only" | "all";
  includeTrace: boolean;
  needsTraceJoin: boolean;
  apiVersion?: "v1" | "v2";
}) => {
  const appliedScoresFilter = scoresFilter.apply();
  const appliedTracesFilter = tracesFilter.apply();

  const query = `
      SELECT
        count(*) as count
      FROM
        scores s
          ${needsTraceJoin ? "LEFT JOIN traces t ON s.trace_id = t.id AND s.project_id = t.project_id" : ""}
      WHERE
        s.project_id = $1
      AND (
        ${scoreScope === "traces_only" ? "" : "s.trace_id IS NULL OR "}
        (s.trace_id IS NOT NULL AND (${needsTraceJoin ? "t.id" : "s.trace_id"}) IN (
          SELECT DISTINCT ON (inner_s.id, inner_s.project_id)
            ${needsTraceJoin ? "inner_s.trace_id" : "inner_s.trace_id"}
          FROM
            scores inner_s
          WHERE
            inner_s.project_id = $1
            ${appliedScoresFilter.query ? `AND ${appliedScoresFilter.query}` : ""}
            ${scoreScope === "traces_only" ? "AND inner_s.session_id IS NULL" : ""}
          ORDER BY
            inner_s.id, inner_s.project_id, inner_s.timestamp DESC
        ))
      )
      ${appliedScoresFilter.query ? `AND ${appliedScoresFilter.query}` : ""}
      ${tracesFilter.length() > 0 ? `AND ${appliedTracesFilter.query}` : ""}
      `;

  return measureAndReturn({
    operationName: "_handleGetScoresCountForPublicApi",
    projectId,
    input: {
      params: [
        projectId,
        ...appliedScoresFilter.params,
        ...appliedTracesFilter.params,
      ],
      tags: {
        feature: "scoring",
        type: "score",
        projectId,
        scoreScope,
        operation_name: "_handleGetScoresCountForPublicApi",
        includeTrace: includeTrace.toString(),
        ...(apiVersion ? { api_version: apiVersion } : {}),
      },
    },
    fn: async (input) => {
      const records = await queryPg<{ count: string }>({
        query,
        params: input.params,
        tags: input.tags,
      });
      return records.map((record) => Number(record.count)).shift();
    },
  });
};

// ─── v3 public-API score query helpers ────────────────────────────────────────

type ListFilterParams = {
  id?: string[];
  name?: string[];
  source?: string[];
  dataType?: string[];
  environment?: string[];
  configId?: string[];
  queueId?: string[];
  authorUserId?: string[];
  value?: string[];
  valueMin?: number;
  valueMax?: number;
  traceId?: string[];
  sessionId?: string[];
  observationId?: string[];
  experimentId?: string[];
  fromTimestamp?: Date;
  toTimestamp?: Date;
};

const CORE_COLUMNS_V3 = [
  "s.id as id",
  "s.project_id as project_id",
  "s.timestamp as timestamp",
  "s.environment as environment",
  "s.name as name",
  "s.value as value",
  "s.string_value as string_value",
  "s.long_string_value as long_string_value",
  "s.source as source",
  "s.data_type as data_type",
  "s.created_at as created_at",
  "s.updated_at as updated_at",
  "s.execution_trace_id as execution_trace_id",
];
const DETAILS_COLUMNS_V3 = [
  "s.comment as comment",
  "s.metadata as metadata",
  "s.config_id as config_id",
];
const SUBJECT_COLUMNS_V3 = [
  "s.trace_id as trace_id",
  "s.observation_id as observation_id",
  "s.session_id as session_id",
  "s.dataset_run_id as dataset_run_id",
];
const ANNOTATION_COLUMNS_V3 = [
  "s.author_user_id as author_user_id",
  "s.queue_id as queue_id",
];

export const buildSelectColumns = (fields: ScoreFieldGroupV3[]): string => {
  const selected = [...CORE_COLUMNS_V3];
  if (fields.includes("details")) selected.push(...DETAILS_COLUMNS_V3);
  if (fields.includes("subject")) selected.push(...SUBJECT_COLUMNS_V3);
  if (fields.includes("annotation")) selected.push(...ANNOTATION_COLUMNS_V3);
  return selected.join(",\n    ");
};

export function transformBooleanValueForFilter(v: "true" | "false"): number {
  if (v === "true") return 1;
  if (v === "false") return 0;
  throw new InternalServerError(
    `transformBooleanValueForFilter received unexpected value: ${v}`,
  );
}

function buildDynamicFilters(params: ListFilterParams): {
  query: string;
  params: unknown[];
} {
  const filterList = new PgFilterList();

  type StringOptionFilterKey = Extract<
    keyof ListFilterParams,
    | "id"
    | "name"
    | "source"
    | "dataType"
    | "environment"
    | "configId"
    | "queueId"
    | "authorUserId"
    | "traceId"
    | "sessionId"
    | "observationId"
    | "experimentId"
  >;

  const STRING_OPTIONS_FILTERS: ReadonlyArray<{
    key: StringOptionFilterKey;
    field: string;
  }> = [
    { key: "id", field: "id" },
    { key: "name", field: "name" },
    { key: "source", field: "source" },
    { key: "dataType", field: "data_type" },
    { key: "environment", field: "environment" },
    { key: "configId", field: "config_id" },
    { key: "queueId", field: "queue_id" },
    { key: "authorUserId", field: "author_user_id" },
    { key: "traceId", field: "trace_id" },
    { key: "sessionId", field: "session_id" },
    { key: "observationId", field: "observation_id" },
    { key: "experimentId", field: "dataset_run_id" },
  ];

  for (const { key, field } of STRING_OPTIONS_FILTERS) {
    const values = params[key];
    if (values?.length) {
      filterList.push(
        new PgStringOptionsFilter({
          clickhouseTable: "scores",
          field,
          operator: "any of",
          values,
          tablePrefix: "s",
        }),
      );
    }
  }
  if (params.fromTimestamp !== undefined)
    filterList.push(
      new PgDateTimeFilter({
        clickhouseTable: "scores",
        field: "timestamp",
        operator: ">=",
        value: params.fromTimestamp,
        tablePrefix: "s",
      }),
    );
  if (params.toTimestamp !== undefined)
    filterList.push(
      new PgDateTimeFilter({
        clickhouseTable: "scores",
        field: "timestamp",
        operator: "<",
        value: params.toTimestamp,
        tablePrefix: "s",
      }),
    );
  if (params.valueMin !== undefined)
    filterList.push(
      new PgNumberFilter({
        clickhouseTable: "scores",
        field: "value",
        operator: ">=",
        value: params.valueMin,
        tablePrefix: "s",
      }),
    );
  if (params.valueMax !== undefined)
    filterList.push(
      new PgNumberFilter({
        clickhouseTable: "scores",
        field: "value",
        operator: "<=",
        value: params.valueMax,
        tablePrefix: "s",
      }),
    );

  const compiled = filterList.apply();

  const extraClauses: string[] = [];
  const extraParams: unknown[] = [];

  if (params.value?.length && params.dataType?.length === 1) {
    const dt = params.dataType[0] as ScoreDataTypeType;

    switch (dt) {
      case ScoreDataTypeEnum.NUMERIC: {
        const numericValues = params.value.map((v) => {
          const n = Number(v);
          if (!Number.isFinite(n)) {
            throw new InternalServerError(
              `NUMERIC value filter received non-finite value: ${v}`,
            );
          }
          return n;
        });
        extraClauses.push(`s.value = ANY(ARRAY[${numericValues.map((_v, i) => `$${compiled.params.length + extraParams.length + i + 1}::float8`).join(", ")}])`);
        extraParams.push(...numericValues);
        break;
      }
      case ScoreDataTypeEnum.BOOLEAN: {
        const boolValues = params.value.map((v) =>
          transformBooleanValueForFilter(v as "true" | "false"),
        );
        extraClauses.push(`s.value = ANY(ARRAY[${boolValues.map((_v, i) => `$${compiled.params.length + extraParams.length + i + 1}::float8`).join(", ")}])`);
        extraParams.push(...boolValues);
        break;
      }
      case ScoreDataTypeEnum.CATEGORICAL: {
        extraClauses.push(`s.string_value = ANY($${compiled.params.length + extraParams.length + 1}::text[])`);
        extraParams.push(params.value);
        break;
      }
      case ScoreDataTypeEnum.TEXT:
      case ScoreDataTypeEnum.CORRECTION:
        throw new InternalServerError(
          `value filter with dataType=${dt} should have been rejected by handler validation`,
        );
      default: {
        const _exhaustiveCheck: never = dt;
        throw new InternalServerError(
          `value filter received unknown dataType: ${_exhaustiveCheck as string}`,
        );
      }
    }
  }

  const allClauses = [compiled.query, ...extraClauses]
    .filter(Boolean)
    .join(" AND ");

  return { query: allClauses, params: [...compiled.params, ...extraParams] };
}

const buildV3ListQuery = (
  withCursor: boolean,
  fields: ScoreFieldGroupV3[],
  filterClause: string,
  paramsStart: number,
) => `
  SELECT DISTINCT ON (s.id, s.project_id)
    ${buildSelectColumns(fields)}
  FROM scores s
  WHERE s.project_id = $1
  ${
    withCursor
      ? `AND (s.timestamp, s.id) < ($${paramsStart}::timestamptz, $${paramsStart + 1}) AND s.timestamp <= $${paramsStart}::timestamptz`
      : ""
  }
  ${filterClause ? `AND ${filterClause.replace(/\$(\d+)/g, (_m: string, n: string) => `$${parseInt(n) + paramsStart + (withCursor ? 2 : 0) - 1}`)}` : ""}
  ORDER BY s.id, s.project_id, s.timestamp DESC, s.id DESC, s.updated_at DESC
  LIMIT $${paramsStart + (withCursor ? 2 : 0)}
`;

export function polymorphicValueForV3(score: {
  dataType: ScoreDataTypeType;
  value: number;
  stringValue?: string | null;
  longStringValue?: string | null;
}): number | boolean | string {
  switch (score.dataType) {
    case ScoreDataTypeEnum.NUMERIC:
      return score.value;
    case ScoreDataTypeEnum.BOOLEAN:
      return score.value === 1;
    case ScoreDataTypeEnum.CATEGORICAL:
    case ScoreDataTypeEnum.TEXT:
      if (score.stringValue == null) {
        throw new InternalServerError(
          `Score with dataType ${score.dataType} is missing its stringValue`,
        );
      }
      return score.stringValue;
    case ScoreDataTypeEnum.CORRECTION:
      if (score.longStringValue == null) {
        throw new InternalServerError(
          "Score with dataType CORRECTION is missing its longStringValue",
        );
      }
      return score.longStringValue;
    default: {
      const _exhaustiveCheck: never = score.dataType;
      throw new InternalServerError(
        `Score has unknown dataType: ${_exhaustiveCheck as string}`,
      );
    }
  }
}

function deriveSubjectForV3(
  score: ScoreDomain,
):
  | { kind: "observation"; id: string; traceId?: string }
  | { kind: "trace" | "session" | "experiment"; id: string } {
  if (score.datasetRunId) {
    return { kind: "experiment", id: score.datasetRunId };
  }
  if (score.observationId) {
    return {
      kind: "observation",
      id: score.observationId,
      ...(score.traceId ? { traceId: score.traceId } : {}),
    };
  }
  if (score.sessionId) {
    return { kind: "session", id: score.sessionId };
  }
  if (!score.traceId) {
    throw new InternalServerError(
      `Score ${score.id} has kind=trace but missing traceId`,
    );
  }
  return { kind: "trace", id: score.traceId };
}

function domainToV3Shared(
  score: ScoreDomain,
  fields: ScoreFieldGroupV3[],
): APIScoreV3 {
  return {
    id: score.id,
    projectId: score.projectId,
    name: score.name,
    dataType: score.dataType,
    value: polymorphicValueForV3({
      dataType: score.dataType,
      value: score.value,
      stringValue: score.stringValue as string | null | undefined,
      longStringValue: score.longStringValue as string | null | undefined,
    }),
    source: score.source,
    timestamp: score.timestamp,
    environment: score.environment,
    createdAt: score.createdAt,
    updatedAt: score.updatedAt,
    ...(fields.includes("details")
      ? {
          comment: score.comment,
          configId: score.configId,
          metadata: score.metadata,
        }
      : {}),
    ...(fields.includes("annotation")
      ? {
          authorUserId: score.authorUserId,
          queueId: score.queueId,
        }
      : {}),
    ...(fields.includes("subject")
      ? { subject: deriveSubjectForV3(score) }
      : {}),
  } as APIScoreV3;
}

export async function listScoresV3ForPublicApi(
  params: {
    projectId: string;
    limit: number;
    cursor?: ScoresCursorV3Type;
    fields: ScoreFieldGroupV3[];
  } & ListFilterParams,
): Promise<{ data: APIScoreV3[]; cursor?: string }> {
  const { query: filterClause, params: filterParams } =
    buildDynamicFilters(params);

  return measureAndReturn({
    operationName: "listScoresV3ForPublicApi",
    projectId: params.projectId,
    input: {
      params: {
        projectId: params.projectId,
        limit: params.limit + 1,
        ...(params.cursor && {
          lastTimestamp: toPgTimestamp(params.cursor.lastTimestamp),
          lastId: params.cursor.lastId,
        }),
        ...Object.fromEntries(filterParams.map((v, i) => [`f${i}`, v])),
      },
      tags: {
        feature: "scoring",
        type: "score",
        projectId: params.projectId,
        operation_name: "listScoresV3ForPublicApi",
        api_version: "v3",
      },
    },
    fn: async (input) => {
      const cursorParams: unknown[] = params.cursor
        ? [toPgTimestamp(params.cursor.lastTimestamp), params.cursor.lastId]
        : [];
      const baseParams: unknown[] = [params.projectId];

      // params order: project_id, cursor_ts, cursor_id, filter_params, limit
      const allQParams: unknown[] = [
        ...baseParams,
        ...cursorParams,
        ...filterParams,
        params.limit + 1,
      ];

      const records = await queryPg<ScoreRecordReadType>({
        query: buildV3ListQuery(
          Boolean(params.cursor),
          params.fields,
          filterClause,
          2, // paramsStart: after project_id ($1)
        ),
        params: allQParams,
        tags: input.tags,
      });

      const hasMore = records.length > params.limit;
      const pageRecords = hasMore ? records.slice(0, params.limit) : records;

      let nextCursor: string | undefined;
      if (hasMore && pageRecords.length > 0) {
        const last = pageRecords[pageRecords.length - 1];
        nextCursor = encodeCursorV3({
          v: 1,
          lastTimestamp: new Date(last.timestamp),
          lastId: last.id,
        });
      }

      const items: APIScoreV3[] = [];
      for (const row of pageRecords) {
        try {
          items.push(
            domainToV3Shared(
              convertClickhouseScoreToDomain(row),
              params.fields,
            ),
          );
        } catch (error) {
          logger.error("v3 score row dropped from response: conversion error", {
            error,
            scoreId: row.id,
            projectId: params.projectId,
          });
        }
      }
      return {
        data: filterAndValidateV3GetScoreList(items, (error) => {
          logger.error(
            "v3 score row dropped from response: schema validation error",
            {
              issues: error.issues,
              projectId: params.projectId,
            },
          );
        }),
        cursor: nextCursor,
      };
    },
  });
}
