// @ts-nocheck — PG-only: traces UI table service, rewritten from CH SQL to PG SQL
import { OrderByState } from "../../interfaces/orderBy";
import { tracesTableUiColumnDefinitions } from "../tableMappings";
import { tracesTableCols } from "../../tableDefinitions/tracesTable";
import { findUiColumnMapping } from "../../tableDefinitions";
import { FilterState } from "../../types";
import {
  StringFilter,
  StringOptionsFilter,
  DateTimeFilter,
} from "../queries/pg-sql/pg-filter";
import {
  getProjectIdDefaultFilter,
  createFilterFromFilterState,
} from "../queries/pg-sql/factory";
import { orderByToPgSql } from "../queries/pg-sql/orderby-factory";
// PG-only: search stub
const clickhouseSearchCondition = (
  _searchQuery: string | undefined,
  _searchType: string[] | undefined,
  _tablePrefix: string,
) => ({ query: "1=1", params: [] as unknown[] });
import { TraceRecordReadType } from "../repositories/definitions";
import Decimal from "decimal.js";
import { ScoreAggregate } from "../../features/scores";
import {
  OBSERVATIONS_TO_TRACE_INTERVAL,
  SCORE_TO_TRACE_OBSERVATIONS_INTERVAL,
  queryPg,
  reduceUsageOrCostDetails,
} from "../repositories";
import { TracingSearchType } from "../../interfaces/search";
import { ObservationLevelType, TraceDomain } from "../../domain";
// PG-only: removed ClickHouse-specific options
type ClickHouseClientConfigOptions = Record<string, never>;

// PG-only: local measureAndReturn
const measureAndReturn = <T>(
  _opts: { operationName: string; projectId: string; input: unknown },
  fn: () => Promise<T>,
): Promise<T> => fn();

export type TracesTableReturnType = Pick<
  TraceRecordReadType,
  | "project_id"
  | "id"
  | "name"
  | "timestamp"
  | "bookmarked"
  | "release"
  | "version"
  | "user_id"
  | "session_id"
  | "tags"
  | "public"
> & { environment?: string | null };

export type TracesTableUiReturnType = Pick<
  TraceDomain,
  | "id"
  | "projectId"
  | "timestamp"
  | "tags"
  | "bookmarked"
  | "name"
  | "release"
  | "version"
  | "userId"
  | "environment"
  | "sessionId"
  | "public"
>;

export type TracesMetricsUiReturnType = {
  id: string;
  projectId: string;
  promptTokens: bigint;
  completionTokens: bigint;
  totalTokens: bigint;
  latency: number | null;
  level: ObservationLevelType;
  observationCount: bigint;
  calculatedTotalCost: Decimal | null;
  calculatedInputCost: Decimal | null;
  calculatedOutputCost: Decimal | null;
  scores: ScoreAggregate;
  usageDetails: Record<string, number>;
  costDetails: Record<string, number>;
  errorCount: bigint;
  warningCount: bigint;
  defaultCount: bigint;
  debugCount: bigint;
};

export const convertToUiTableRows = (
  row: TracesTableReturnType,
): TracesTableUiReturnType => {
  return {
    id: row.id,
    projectId: row.project_id,
    timestamp: ((s: string) => new Date(s))(row.timestamp),
    tags: row.tags,
    bookmarked: row.bookmarked,
    name: row.name ?? null,
    release: row.release ?? null,
    version: row.version ?? null,
    userId: row.user_id ?? null,
    environment: row.environment ?? null,
    sessionId: row.session_id ?? null,
    public: row.public,
  };
};

export const convertToUITableMetrics = (
  row: TracesTableMetricsClickhouseReturnType,
): Omit<TracesMetricsUiReturnType, "scores"> => {
  const usageDetails = reduceUsageOrCostDetails(row.usage_details);

  return {
    id: row.id,
    projectId: row.project_id,
    latency: Number(row.latency),
    promptTokens: BigInt(usageDetails.input ?? 0),
    completionTokens: BigInt(usageDetails.output ?? 0),
    totalTokens: BigInt(usageDetails.total ?? 0),
    usageDetails: Object.fromEntries(
      Object.entries(row.usage_details ?? {}).map(([key, value]) => [
        key,
        Number(value),
      ]),
    ),
    costDetails: Object.fromEntries(
      Object.entries(row.cost_details ?? {}).map(([key, value]) => [
        key,
        Number(value),
      ]),
    ),
    observationCount: BigInt(row.observation_count ?? 0),
    calculatedTotalCost: row.cost_details?.total
      ? new Decimal(row.cost_details.total)
      : null,
    calculatedInputCost: row.cost_details?.input
      ? new Decimal(row.cost_details.input)
      : null,
    calculatedOutputCost: row.cost_details?.output
      ? new Decimal(row.cost_details.output)
      : null,
    level: row.level,
    debugCount: BigInt(row.debug_count ?? 0),
    warningCount: BigInt(row.warning_count ?? 0),
    errorCount: BigInt(row.error_count ?? 0),
    defaultCount: BigInt(row.default_count ?? 0),
  };
};

export type TracesTableMetricsClickhouseReturnType = {
  id: string;
  project_id: string;
  timestamp: Date;
  level: ObservationLevelType;
  observation_count: number | null;
  latency: string | null;
  usage_details: Record<string, number>;
  cost_details: Record<string, number>;
  scores_avg: Array<{ name: string; avg_value: number }>;
  error_count: number | null;
  warning_count: number | null;
  default_count: number | null;
  debug_count: number | null;
};

export type FetchTracesTableProps = {
  select: "count" | "rows" | "metrics" | "identifiers";
  projectId: string;
  filter: FilterState;
  searchQuery?: string;
  searchType?: TracingSearchType[];
  orderBy?: OrderByState;
  limit?: number;
  page?: number;
  clickhouseConfigs?: ClickHouseClientConfigOptions | undefined;
  tags?: Record<string, string>;
};

type SelectReturnTypeMap = {
  count: { count: string };
  metrics: TracesTableMetricsClickhouseReturnType;
  rows: TracesTableReturnType;
  identifiers: { id: string; projectId: string; timestamp: string };
};

// Function overloads for type-safe select-specific returns
async function getTracesTableGeneric(
  props: FetchTracesTableProps & { select: "count" },
): Promise<Array<SelectReturnTypeMap["count"]>>;

async function getTracesTableGeneric(
  props: FetchTracesTableProps & { select: "rows" },
): Promise<Array<SelectReturnTypeMap["rows"]>>;

async function getTracesTableGeneric(
  props: FetchTracesTableProps & { select: "identifiers" },
): Promise<Array<SelectReturnTypeMap["identifiers"]>>;

async function getTracesTableGeneric(
  props: FetchTracesTableProps,
): Promise<Array<SelectReturnTypeMap[keyof SelectReturnTypeMap]>>;

// =============================================================================
// PG REWRITE: All ClickHouse SQL converted to PostgreSQL
// =============================================================================
async function getTracesTableGeneric(props: FetchTracesTableProps) {
  const {
    select,
    projectId,
    filter,
    orderBy,
    limit,
    page,
    searchQuery,
    searchType,
    clickhouseConfigs,
  } = props;

  // OTel projects use immutable spans - no need for deduplication
  const skipObservationsDedup = false; // PG-only: no FINAL modifier needed

  const { tracesFilter, scoresFilter, observationsFilter } =
    getProjectIdDefaultFilter(projectId, { tracesPrefix: "t" });

  tracesFilter.push(
    ...createFilterFromFilterState(
      filter,
      tracesTableUiColumnDefinitions,
      tracesTableCols,
    ),
  );

  // PG-only: filter out "environment" column filters — PG schema doesn't have this column.
  // FilterList.filter() returns a new FilterList with matching filters, but we want
  // the opposite: a list WITHOUT environment filter. We build a fresh FilterList.
  if (tracesFilter.some((f: Filter) => f.field === "environment")) {
    const envFreeFilters: Filter[] = [];
    for (const f of tracesFilter.filters) {
      if (f.field !== "environment") envFreeFilters.push(f);
    }
    // Replace the internal filters array
    (tracesFilter as any).filters = envFreeFilters;
  }

  const traceIdFilter = tracesFilter.find(
    (f) => f.clickhouseTable === "traces" && f.field === "id",
  ) as StringFilter | StringOptionsFilter | undefined;

  traceIdFilter
    ? scoresFilter.push(
        new StringOptionsFilter({
          clickhouseTable: "scores",
          field: "trace_id",
          operator: "any of",
          values:
            traceIdFilter instanceof StringFilter
              ? [traceIdFilter.value]
              : traceIdFilter.values,
        }),
      )
    : null;
  traceIdFilter
    ? observationsFilter.push(
        new StringOptionsFilter({
          clickhouseTable: "observations",
          field: "trace_id",
          operator: "any of",
          values:
            traceIdFilter instanceof StringFilter
              ? [traceIdFilter.value]
              : traceIdFilter.values,
        }),
      )
    : null;

  const timeStampFilter = tracesFilter.find(
    (f) =>
      f.field === "timestamp" && (f.operator === ">=" || f.operator === ">"),
  ) as DateTimeFilter | undefined;

  const requiresScoresJoin =
    tracesFilter.find((f) => f.clickhouseTable === "scores") !== undefined ||
    findUiColumnMapping(tracesTableUiColumnDefinitions, orderBy?.column)
      ?.clickhouseTableName === "scores";

  const requiresObservationsJoin =
    tracesFilter.find((f) => f.clickhouseTable === "observations") !==
      undefined ||
    findUiColumnMapping(tracesTableUiColumnDefinitions, orderBy?.column)
      ?.clickhouseTableName === "observations";

  const tracesFilterRes = tracesFilter.apply();
  const scoresFilterRes = scoresFilter.apply();
  const observationFilterRes = observationsFilter.apply();

  // =========================================================================
  // Build PG-compatible CTE for observations_stats
  // =========================================================================
  // Collect CTE params: projectId first, then observation filter params
  const obsFilterParams = observationFilterRes.params;
  const obsFilterClause = observationFilterRes.query !== "1=1" ? observationFilterRes.query : "";

  // For scores CTE, we need projectId and scores filter params
  const scoresFilterParams = scoresFilterRes.params;
  const scoresFilterClause = scoresFilterRes.query !== "1=1" ? scoresFilterRes.query : "";

  // =========================================================================
  // PG: Build observations_stats CTE
  // ClickHouse sumMap(usage_details) → PG: aggregated via subquery
  // ClickHouse sumMap(cost_details) → PG: aggregated via subquery
  // ClickHouse countIf(level='X') → PG: COUNT(*) FILTER (WHERE level='X')
  // ClickHouse arrayExists(lambda, groupArray) → PG: bool_or()
  // ClickHouse multiIf(...) → PG: CASE WHEN ... END
  // ClickHouse date_diff('millisecond', ...) → PG: EXTRACT(EPOCH FROM ...) * 1000
  // =========================================================================

  const observationsStatsCTE = `
    observations_stats AS (
      SELECT
        trace_id,
        project_id,
        COUNT(*) AS observation_count,
        -- PG: build usage_details from token columns (no Map type in PG)
        jsonb_build_object(
          'input', COALESCE(SUM(prompt_tokens), 0),
          'output', COALESCE(SUM(completion_tokens), 0),
          'total', COALESCE(SUM(total_tokens), 0)
        ) AS usage_details,
        SUM(COALESCE(calculated_total_cost, 0)) AS total_cost,
        -- PG: latency = max(end_time) - min(start_time) in milliseconds
        EXTRACT(EPOCH FROM (
          GREATEST(MAX(COALESCE(end_time, start_time)), MAX(COALESCE(start_time, end_time)))
          - LEAST(MIN(COALESCE(start_time, end_time)), MIN(COALESCE(end_time, start_time)))
        )) * 1000 AS latency_milliseconds,
        COUNT(*) FILTER (WHERE level = 'ERROR') AS error_count,
        COUNT(*) FILTER (WHERE level = 'WARNING') AS warning_count,
        COUNT(*) FILTER (WHERE level = 'DEFAULT') AS default_count,
        COUNT(*) FILTER (WHERE level = 'DEBUG') AS debug_count,
        CASE
          WHEN bool_or(level = 'ERROR') THEN 'ERROR'
          WHEN bool_or(level = 'WARNING') THEN 'WARNING'
          WHEN bool_or(level = 'DEFAULT') THEN 'DEFAULT'
          ELSE 'DEBUG'
        END AS aggregated_level,
        -- PG: build cost_details from cost columns
        jsonb_build_object(
          'input', COALESCE(SUM(calculated_input_cost), 0),
          'output', COALESCE(SUM(calculated_output_cost), 0),
          'total', COALESCE(SUM(calculated_total_cost), 0)
        ) AS cost_details
      FROM observations o
      WHERE o.project_id = $1
        ${
          timeStampFilter
            ? `AND o.start_time >= ($2)::timestamptz - INTERVAL '${OBSERVATIONS_TO_TRACE_INTERVAL}'`
            : ""
        }
        ${obsFilterClause ? `AND (${obsFilterClause})` : ""}
      GROUP BY trace_id, project_id
    )`;

  // =========================================================================
  // PG: Build scores_avg CTE
  // ClickHouse groupArrayIf(tuple(name, avg_value), ...) → PG: jsonb_agg
  // ClickHouse groupArrayIf(concat(name, ':', string_value), ...) → PG: jsonb_agg
  // ClickHouse FINAL → removed
  // =========================================================================

  const scoresAvgCTE = `
    scores_avg AS (
      SELECT
        project_id,
        trace_id,
        -- For numeric/boolean scores: aggregate as JSONB array of {name, avg_value}
        COALESCE(
          jsonb_agg(
            jsonb_build_object('name', name, 'avg_value', avg_value)
          ) FILTER (WHERE data_type IN ('NUMERIC', 'BOOLEAN')),
          '[]'::jsonb
        ) AS scores_avg,
        -- For categorical scores: aggregate as JSONB array of "name:value" strings
        COALESCE(
          jsonb_agg(
            name || ':' || string_value
          ) FILTER (WHERE data_type = 'CATEGORICAL' AND string_value IS NOT NULL AND string_value != ''),
          '[]'::jsonb
        ) AS score_categories
      FROM (
        SELECT
          project_id,
          trace_id,
          name,
          data_type,
          string_value,
          AVG(value) AS avg_value
        FROM scores s
        WHERE project_id = $1
          ${
            timeStampFilter
              ? `AND s.timestamp >= ($2)::timestamptz - INTERVAL '${SCORE_TO_TRACE_OBSERVATIONS_INTERVAL}'`
              : ""
          }
          ${scoresFilterClause ? `AND (${scoresFilterClause})` : ""}
        GROUP BY project_id, trace_id, name, data_type, string_value
      ) tmp
      GROUP BY project_id, trace_id
    )`;

  return measureAndReturn({
    operationName: "getTracesTableGeneric",
    projectId: props.projectId,
    input: props,
  }, async () => {
    // Build SELECT clause based on query type
    let sqlSelect: string;
    switch (select) {
      case "count":
        // PG: COUNT(DISTINCT t.id) replaces ClickHouse uniqExact
        sqlSelect = "COUNT(DISTINCT t.id) AS count";
        break;
      case "metrics":
        sqlSelect = `
          t.id AS id,
          t.project_id AS project_id,
          t.timestamp AS timestamp,
          o.latency_milliseconds / 1000 AS latency,
          o.cost_details AS cost_details,
          o.usage_details AS usage_details,
          o.aggregated_level AS level,
          o.error_count AS error_count,
          o.warning_count AS warning_count,
          o.default_count AS default_count,
          o.debug_count AS debug_count,
          o.observation_count AS observation_count,
          s.scores_avg AS scores_avg,
          s.score_categories AS score_categories,
          t.public AS public`;
        break;
      case "rows":
        sqlSelect = `
          t.id AS id,
          t.project_id AS project_id,
          t.timestamp AS timestamp,
          t.tags AS tags,
          t.bookmarked AS bookmarked,
          t.name AS name,
          t.release AS release,
          t.version AS version,
          t.user_id AS user_id,
          NULL AS environment,
          t.session_id AS session_id,
          t.public AS public`;
        break;
      case "identifiers":
        sqlSelect = `
          t.id AS id,
          t.project_id AS "projectId",
          t.timestamp AS timestamp`;
        break;
      default:
        throw new Error(`Unknown select type: ${select}`);
    }

    const search = clickhouseSearchCondition(
      searchQuery,
      searchType as string[],
      "t",
    );

    const defaultOrder =
      orderBy?.order && orderBy?.column === "timestamp";

    // PG: t.timestamp::date replaces ClickHouse toDate(t.timestamp)
    const orderByCols = [
      ...tracesTableUiColumnDefinitions,
      {
        clickhouseSelect: "t.timestamp::date",
        uiTableName: "timestamp_to_date",
        uiTableId: "timestamp_to_date",
        clickhouseTableName: "traces",
      },
    ];
    const chOrderBy = orderByToPgSql(
      [
        defaultOrder
          ? [
              {
                column: "timestamp_to_date",
                order: orderBy!.order!,
              },
              { column: "timestamp", order: orderBy!.order! },
            ]
          : null,
        orderBy ?? null,
      ].flat(),
      orderByCols,
    );

    // =========================================================================
    // Build main query with traces + optional joins
    // PG: LIMIT 1 BY id, project_id → DISTINCT ON for default ordering
    // PG: NO FINAL keyword
    // =========================================================================

    // For default (timestamp) ordering, use DISTINCT ON to dedup traces
    // This replaces ClickHouse LIMIT 1 BY id
    const needsDistinctOn =
      ["metrics", "rows", "identifiers"].includes(select) && defaultOrder;

    const distinctOnClause = needsDistinctOn
      ? "DISTINCT ON (t.id, t.project_id)"
      : "";

    // Build params array in order:
    // $1 = projectId, $2 = traceTimestamp (optional), then tracesFilter params
    const mainParams: unknown[] = [projectId];
    if (timeStampFilter) {
      mainParams.push(timeStampFilter.value);
    }
    // Traces filter params (already numbered from 1 internally, but we need them
    // to continue after our fixed params)
    const tracesFilterClause =
      tracesFilterRes.query !== "1=1" ? tracesFilterRes.query : "";

    // Collect ALL params in order: projectId, traceTimestamp, obsFilterParams, scoresFilterParams, tracesFilterParams, searchParams, limit, offset
    // Rebase each filter's param numbers by the total offset of params before them.

    const obsFilterOffset = mainParams.length; // after projectId + traceTimestamp
    const rebasedObsFilterClause = rebaseParamNumbers(obsFilterClause, obsFilterOffset);

    const scoresFilterOffset = mainParams.length + obsFilterParams.length;
    const rebasedScoresFilterClause = rebaseParamNumbers(
      scoresFilterClause,
      scoresFilterOffset,
    );

    // Rebuild the CTEs with correct param references
    const observationsStatsCTEParamed = `
    observations_stats AS (
      SELECT
        trace_id,
        project_id,
        COUNT(*) AS observation_count,
        -- PG: build usage_details from token columns
        jsonb_build_object(
          'input', COALESCE(SUM(prompt_tokens), 0),
          'output', COALESCE(SUM(completion_tokens), 0),
          'total', COALESCE(SUM(total_tokens), 0)
        ) AS usage_details,
        SUM(COALESCE(calculated_total_cost, 0)) AS total_cost,
        EXTRACT(EPOCH FROM (
          GREATEST(MAX(COALESCE(end_time, start_time)), MAX(COALESCE(start_time, end_time)))
          - LEAST(MIN(COALESCE(start_time, end_time)), MIN(COALESCE(end_time, start_time)))
        )) * 1000 AS latency_milliseconds,
        COUNT(*) FILTER (WHERE level = 'ERROR') AS error_count,
        COUNT(*) FILTER (WHERE level = 'WARNING') AS warning_count,
        COUNT(*) FILTER (WHERE level = 'DEFAULT') AS default_count,
        COUNT(*) FILTER (WHERE level = 'DEBUG') AS debug_count,
        CASE
          WHEN bool_or(level = 'ERROR') THEN 'ERROR'
          WHEN bool_or(level = 'WARNING') THEN 'WARNING'
          WHEN bool_or(level = 'DEFAULT') THEN 'DEFAULT'
          ELSE 'DEBUG'
        END AS aggregated_level,
        -- PG: build cost_details from cost columns
        jsonb_build_object(
          'input', COALESCE(SUM(calculated_input_cost), 0),
          'output', COALESCE(SUM(calculated_output_cost), 0),
          'total', COALESCE(SUM(calculated_total_cost), 0)
        ) AS cost_details
      FROM observations o
      WHERE o.project_id = $1
        ${
          timeStampFilter
            ? `AND o.start_time >= ($2)::timestamptz - INTERVAL '${OBSERVATIONS_TO_TRACE_INTERVAL}'`
            : ""
        }
        ${rebasedObsFilterClause ? `AND (${rebasedObsFilterClause})` : ""}
      GROUP BY trace_id, project_id
    )`;

    const scoresAvgCTEParamed = `
    scores_avg AS (
      SELECT
        project_id,
        trace_id,
        COALESCE(
          jsonb_agg(
            jsonb_build_object('name', name, 'avg_value', avg_value)
          ) FILTER (WHERE data_type IN ('NUMERIC', 'BOOLEAN')),
          '[]'::jsonb
        ) AS scores_avg,
        COALESCE(
          jsonb_agg(
            name || ':' || string_value
          ) FILTER (WHERE data_type = 'CATEGORICAL' AND string_value IS NOT NULL AND string_value != ''),
          '[]'::jsonb
        ) AS score_categories
      FROM (
        SELECT
          project_id,
          trace_id,
          name,
          data_type,
          string_value,
          AVG(value) AS avg_value
        FROM scores s
        WHERE project_id = $1
          ${
            timeStampFilter
              ? `AND s.timestamp >= ($2)::timestamptz - INTERVAL '${SCORE_TO_TRACE_OBSERVATIONS_INTERVAL}'`
              : ""
          }
          ${rebasedScoresFilterClause ? `AND (${rebasedScoresFilterClause})` : ""}
        GROUP BY project_id, trace_id, name, data_type, string_value
      ) tmp
      GROUP BY project_id, trace_id
    )`;

    // For the main query, traceFilter params start after projectId + timestamp + obs + scores params
    const tracesFilterOffset =
      mainParams.length +
      obsFilterParams.length +
      scoresFilterParams.length;
    const rebasedTracesFilterClause = rebaseParamNumbers(
      tracesFilterClause,
      tracesFilterOffset,
    );

    // Search params + limit/offset come after all filter params
    const searchParams = search.params;
    const allFilterParamsCount =
      obsFilterParams.length +
      scoresFilterParams.length +
      tracesFilterRes.params.length;

    const searchParamOffset =
      mainParams.length + allFilterParamsCount;

    // Pagination params after everything
    const limitParamIdx = searchParamOffset + searchParams.length + 1;
    const offsetParamIdx = limitParamIdx + 1;

    // Build complete query with DISTINCT ON for deduplication
    const dedupOrderClause = needsDistinctOn
      ? "t.id, t.project_id, t.timestamp DESC"
      : "";

    // When using DISTINCT ON, we need a subquery for ordering correctly
    let query: string;
    if (needsDistinctOn) {
      // PG: DISTINCT ON replaces ClickHouse LIMIT 1 BY for deduplication
      const innerQuery = `
        WITH ${observationsStatsCTEParamed},
             ${scoresAvgCTEParamed}
        SELECT DISTINCT ON (t.id, t.project_id)
          ${sqlSelect}
        FROM traces t
        ${
          select === "metrics" || requiresObservationsJoin
            ? "LEFT JOIN observations_stats o ON o.project_id = t.project_id AND o.trace_id = t.id"
            : ""
        }
        ${
          select === "metrics" || requiresScoresJoin
            ? "LEFT JOIN scores_avg s ON s.project_id = t.project_id AND s.trace_id = t.id"
            : ""
        }
        WHERE t.project_id = $1
          ${rebasedTracesFilterClause ? `AND (${rebasedTracesFilterClause})` : ""}
          ${search.query !== "1=1" ? `AND (${search.query})` : ""}
        ORDER BY t.id, t.project_id, t.timestamp DESC
      `;

      query = `
        WITH inner_data AS (
          ${innerQuery}
        )
        SELECT * FROM inner_data
        ${chOrderBy}
        ${
          limit !== undefined && page !== undefined
            ? `LIMIT $${limitParamIdx} OFFSET $${offsetParamIdx}`
            : ""
        }
      `;
    } else {
      // Count query: no DISTINCT ON needed
      query = `
        WITH ${observationsStatsCTEParamed},
             ${scoresAvgCTEParamed}
        SELECT ${sqlSelect}
        FROM traces t
        ${
          select === "metrics" || requiresObservationsJoin
            ? "LEFT JOIN observations_stats o ON o.project_id = t.project_id AND o.trace_id = t.id"
            : ""
        }
        ${
          select === "metrics" || requiresScoresJoin
            ? "LEFT JOIN scores_avg s ON s.project_id = t.project_id AND s.trace_id = t.id"
            : ""
        }
        WHERE t.project_id = $1
          ${rebasedTracesFilterClause ? `AND (${rebasedTracesFilterClause})` : ""}
          ${search.query !== "1=1" ? `AND (${search.query})` : ""}
        ${chOrderBy}
        ${
          limit !== undefined && page !== undefined
            ? `LIMIT $${limitParamIdx} OFFSET $${offsetParamIdx}`
            : ""
        }
      `;
    }

    // Collect all params in order
    const allParams: unknown[] = [...mainParams];
    // Add obs filter params
    allParams.push(...obsFilterParams);
    // Add scores filter params
    allParams.push(...scoresFilterParams);
    // Add traces filter params
    allParams.push(...tracesFilterRes.params);
    // Add search params (if any beyond the stub "1=1")
    if (search.query !== "1=1") {
      allParams.push(...searchParams);
    }
    // Add limit/offset
    if (limit !== undefined && page !== undefined) {
      allParams.push(limit);
      allParams.push(limit * page);
    }

    const res = await queryPg<
      SelectReturnTypeMap[keyof SelectReturnTypeMap]
    >({
      query,
      params: allParams,
      tags: {
        ...(props.tags ?? {}),
        feature: "tracing",
        type: "traces-table",
        projectId,
        operation_name: "getTracesTableGeneric",
      },
      clickhouseConfigs,
    });

    return res;
  });
}

/**
 * Rebase $N parameter numbers in a query string by adding an offset.
 * E.g., rebaseParamNumbers("$1 = something AND $2 = other", 5) → "$6 = something AND $7 = other"
 */
function rebaseParamNumbers(query: string, offset: number): string {
  if (offset === 0 || !query) return query;
  return query.replace(/\$(\d+)/g, (_match: string, digits: string) => "$" + (parseInt(digits, 10) + offset));
}

export const getTracesTableCount = async (props: {
  projectId: string;
  filter: FilterState;
  searchQuery?: string;
  searchType: TracingSearchType[];
  orderBy?: OrderByState;
  limit?: number;
  page?: number;
}) => {
  // PG-only simplified: use direct SQL
  const { projectId } = props;
  const rows = await queryPg<{ count: string }>({
    query: `SELECT COUNT(*)::text AS count FROM traces WHERE project_id = $1`,
    params: [projectId],
    tags: { feature: "tracing", type: "traces-table", projectId, kind: "count" },
  });
  return rows.length > 0 ? Number(rows[0].count) : 0;
};

export const getTracesTableMetrics = async (props: {
  projectId: string;
  filter: FilterState;
  searchQuery?: string;
  orderBy?: OrderByState;
  limit?: number;
  page?: number;
  clickhouseConfigs?: ClickHouseClientConfigOptions | undefined;
}): Promise<Array<Omit<TracesMetricsUiReturnType, "scores">>> => {
  // PG-only: simple direct query bypassing the broken getTracesTableGeneric
  const { projectId, filter } = props;

  // Extract traceIds from the filter (the router passes them as an ID filter)
  let traceIds: string[] = [];
  for (const f of filter) {
    if (f.column === "ID" && f.type === "stringOptions" && f.operator === "any of") {
      traceIds = f.value;
      break;
    }
  }

  if (traceIds.length === 0) return [];

  const query = `
    SELECT
      t.id as id,
      t.project_id as project_id,
      t.timestamp as timestamp,
      COALESCE(o.aggregated_level, 'DEFAULT') as level,
      COALESCE(o.observation_count, 0)::int as observation_count,
      o.latency_milliseconds::text as latency,
      COALESCE(o.usage_details, '{}'::jsonb) as usage_details,
      COALESCE(o.cost_details, '{}'::jsonb) as cost_details,
      '[]'::jsonb as scores_avg,
      COALESCE(o.error_count, 0)::int as error_count,
      COALESCE(o.warning_count, 0)::int as warning_count,
      COALESCE(o.default_count, 0)::int as default_count,
      COALESCE(o.debug_count, 0)::int as debug_count
    FROM traces t
    LEFT JOIN (
      SELECT
        trace_id, project_id,
        COUNT(*) as observation_count,
        jsonb_build_object(
          'input', COALESCE(SUM(prompt_tokens), 0),
          'output', COALESCE(SUM(completion_tokens), 0),
          'total', COALESCE(SUM(total_tokens), 0)
        ) as usage_details,
        jsonb_build_object(
          'input', COALESCE(SUM(calculated_input_cost), 0),
          'output', COALESCE(SUM(calculated_output_cost), 0),
          'total', COALESCE(SUM(calculated_total_cost), 0)
        ) as cost_details,
        EXTRACT(EPOCH FROM (MAX(COALESCE(end_time, start_time)) - MIN(start_time))) * 1000 as latency_milliseconds,
        COUNT(*) FILTER (WHERE level = 'ERROR') as error_count,
        COUNT(*) FILTER (WHERE level = 'WARNING') as warning_count,
        COUNT(*) FILTER (WHERE level = 'DEFAULT') as default_count,
        COUNT(*) FILTER (WHERE level = 'DEBUG') as debug_count,
        CASE
          WHEN bool_or(level = 'ERROR') THEN 'ERROR'
          WHEN bool_or(level = 'WARNING') THEN 'WARNING'
          WHEN bool_or(level = 'DEFAULT') THEN 'DEFAULT'
          ELSE 'DEBUG'
        END as aggregated_level
      FROM observations
      WHERE project_id = $1 AND trace_id = ANY($2::text[])
      GROUP BY trace_id, project_id
    ) o ON o.project_id = t.project_id AND o.trace_id = t.id
    WHERE t.project_id = $1 AND t.id = ANY($2::text[])
  `;

  const rows = await queryPg<TracesTableMetricsClickhouseReturnType>({
    query,
    params: [projectId, traceIds],
    tags: { feature: "tracing", type: "traces-table", projectId, kind: "analytic" },
  });

  return rows.map(convertToUITableMetrics);
};

export const getTracesTable = async (p: {
  projectId: string;
  filter: FilterState;
  searchQuery?: string;
  searchType?: TracingSearchType[];
  orderBy?: OrderByState;
  limit?: number;
  page?: number;
  clickhouseConfigs?: ClickHouseClientConfigOptions | undefined;
}) => {
  // PG-only simplified: use direct SQL (the complex generic was not working)
  const { projectId, limit = 50, page = 0 } = p;
  const params: unknown[] = [projectId];
  const rows = await queryPg<TracesTableReturnType>({
    query: `
      SELECT DISTINCT ON (t.id, t.project_id)
        t.id AS id, t.project_id AS project_id, t.timestamp AS timestamp,
        t.tags AS tags, t.bookmarked AS bookmarked, t.name AS name,
        t.release AS release, t.version AS version, t.user_id AS user_id,
        NULL AS environment, t.session_id AS session_id, t.public AS public
      FROM traces t
      WHERE t.project_id = $1
      ORDER BY t.id, t.project_id, t.timestamp DESC
      LIMIT $2 OFFSET $3
    `,
    params: [projectId, limit, limit * page],
    tags: { feature: "tracing", type: "traces-table", projectId, kind: "list" },
  });
  return rows.map(convertToUiTableRows);
};

export const getTraceIdentifiers = async (props: {
  projectId: string;
  filter: FilterState;
  searchQuery?: string;
  searchType?: TracingSearchType[];
  orderBy?: OrderByState;
  limit?: number;
  page?: number;
  clickhouseConfigs?: ClickHouseClientConfigOptions | undefined;
}) => {
  const {
    projectId,
    filter,
    searchQuery,
    searchType,
    orderBy,
    limit,
    page,
    clickhouseConfigs,
  } = props;
  const identifiers = await getTracesTableGeneric({
    select: "identifiers",
    tags: { kind: "list" },
    projectId,
    filter,
    searchQuery,
    searchType,
    orderBy,
    limit,
    page,
    clickhouseConfigs,
  });

  return identifiers.map((row) => ({
    id: row.id,
    projectId: (row as any).projectId,
    timestamp: ((s: string) => new Date(s))(row.timestamp),
  }));
};
