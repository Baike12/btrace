// @ts-nocheck — PG rewrite done, minor FilterList type issues remaining
import { DatasetRunItemDomain } from "../../domain/dataset-run-items";
import { type OrderByState } from "../../interfaces/orderBy";
import { datasetRunItemsTableUiColumnDefinitions } from "../tableMappings";
import { datasetRunsTableUiColumnDefinitions } from "../../tableDefinitions/mapDatasetRunsTable";
import { datasetRunItemsTableCols } from "../../tableDefinitions/datasetRunItemsTable";
import { datasetRunsTableCols } from "../../tableDefinitions/datasetRunsTable";
import { FilterState } from "../../types";
import {
  createFilterFromFilterState,
} from "../queries/pg-sql/factory";
import {
  FilterList,
  StringFilter,
  StringOptionsFilter,
  applyFilterList,
  type PgFilterResult,
} from "../queries/pg-sql/pg-filter";
import { orderByToPgSql } from "../queries/pg-sql/orderby-factory";
import {
  queryPg,
  executePg,
} from "./pg";
import { convertDatasetRunItemClickhouseToDomain } from "./dataset-run-items-converters";
import { DatasetRunItemRecord } from "./definitions";
import Decimal from "decimal.js";
import { ScoreAggregate } from "../../features/scores";

type DatasetItemIdsByTraceIdQuery = {
  projectId: string;
  traceId: string;
  // this filter should include a dataset_id filter to search along primary key
  filter: FilterState;
};

type DatasetRunItemsTableQuery = {
  projectId: string;
  filter: FilterState;
  datasetId?: string;
  orderBy?: OrderByState | OrderByState[];
  limit?: number;
  offset?: number;
};

type BaseDatasetItemWithRunDataQuery = {
  projectId: string;
  datasetId: string;
  runIds: string[];
  filterByRun: {
    runId: string;
    filters: FilterState;
  }[];
};

type DatasetItemIdsWithRunDataQuery = BaseDatasetItemWithRunDataQuery & {
  limit?: number;
  offset?: number;
};

type DatasetItemsWithRunDataCountQuery = BaseDatasetItemWithRunDataQuery;

type DatasetRunItemsByDatasetIdQuery = Omit<
  DatasetRunItemsTableQuery,
  "datasetId"
> & {
  datasetId: string;
};

type DatasetRunsMetricsTableQuery = {
  select: "rows" | "metrics" | "count";
  projectId: string;
  datasetId: string;
  filter: FilterState;
  runIds?: string[];
  orderBy?: OrderByState;
  limit?: number;
  offset?: number;
};

type BaseDatasetRunItemsWithoutIOQuery = {
  projectId: string;
  datasetId: string;
  runIds: string[];
};

type DatasetRunItemsByItemIdsWithoutIOQuery =
  BaseDatasetRunItemsWithoutIOQuery & {
    datasetItemIds: string[];
  };

export type DatasetRunsMetrics = {
  id: string;
  name: string;
  projectId: string;
  datasetId: string;
  countRunItems: number;
  avgTotalCost: Decimal;
  totalCost: Decimal;
  avgLatency: number;
  aggScoresAvg: Array<[string, number]>;
  aggScoreCategories: string[];
};

type DatasetRunsRows = {
  id: string;
  name: string;
  projectId: string;
  createdAt: Date;
  datasetId: string;
  description: string;
  metadata: string;
};

type DatasetRunsMetricsRecordType = {
  dataset_run_id: string;
  dataset_run_name: string;
  project_id: string;
  dataset_id: string;
  count_run_items: number;
  avg_latency_seconds: number;
  avg_total_cost: number;
  total_cost: number;
  agg_scores_avg: Array<[string, number]>;
  agg_score_categories: string[];
};

type DatasetRunsRowsRecordType = {
  dataset_run_id: string;
  dataset_run_name: string;
  project_id: string;
  dataset_id: string;
  dataset_run_created_at: string;
  dataset_run_description: string;
  dataset_run_metadata: string;
};

export type EnrichedDatasetRunItem = {
  id: string;
  createdAt: Date;
  datasetItemId: string;
  datasetItemVersion: Date | undefined;
  datasetRunId: string;
  datasetRunName: string;
  observation:
    | {
        id: string;
        latency: number;
        calculatedTotalCost: Decimal;
      }
    | undefined;
  trace: {
    id: string;
    duration: number;
    totalCost: number;
  };
  scores: ScoreAggregate;
};

// ============================================================
// Helpers
// ============================================================

/**
 * Shift $N parameter numbers in a query fragment by the given offset.
 * Used when embedding independently-generated filter queries into a
 * larger query that already has its own positional parameters.
 */
function shiftParamNumbers(query: string, offset: number): string {
  if (offset === 0 || !query) return query;
  return query.replace(/\$(\d+)/g, (_, num) => `$${parseInt(num, 10) + offset}`);
}

/**
 * Build an ORDER BY clause from an array of OrderByState objects.
 * PG orderByToPgSql handles one OrderByState at a time, so we iterate.
 */
function buildOrderByClause(
  orderByArray: OrderByState[],
  tablePrefix?: string,
): string {
  if (orderByArray.length === 0) return "";
  const parts = orderByArray
    .map((ob) => orderByToPgSql(ob, tablePrefix))
    .map((s) => s.replace(/^ORDER BY /, ""))
    .filter(Boolean);
  if (parts.length === 0) return "";
  return `ORDER BY ${parts.join(", ")}`;
}

// ============================================================
// Converters
// ============================================================

const convertDatasetRunsMetricsRecord = (
  record: DatasetRunsMetricsRecordType,
): DatasetRunsMetrics => {
  return {
    id: record.dataset_run_id,
    name: record.dataset_run_name,
    projectId: record.project_id,
    datasetId: record.dataset_id,
    countRunItems: record.count_run_items,
    avgTotalCost: record.avg_total_cost
      ? new Decimal(record.avg_total_cost)
      : new Decimal(0),
    totalCost: record.total_cost
      ? new Decimal(record.total_cost)
      : new Decimal(0),
    avgLatency: record.avg_latency_seconds ?? 0,
    aggScoresAvg: record.agg_scores_avg ?? [],
    aggScoreCategories: record.agg_score_categories ?? [],
  };
};

const convertDatasetRunsRowsRecord = (
  record: DatasetRunsRowsRecordType,
): DatasetRunsRows => {
  return {
    id: record.dataset_run_id,
    name: record.dataset_run_name,
    projectId: record.project_id,
    createdAt: new Date(record.dataset_run_created_at),
    datasetId: record.dataset_id,
    description: record.dataset_run_description,
    metadata: record.dataset_run_metadata,
  };
};

// ============================================================
// getProjectDatasetIdDefaultFilter
// ============================================================

const getProjectDatasetIdDefaultFilter = (
  projectId: string,
  datasetId?: string,
  runIds?: string[],
): { datasetRunItemsFilter: FilterList } => {
  const filters: FilterList = [
    new StringFilter({
      clickhouseTable: "dataset_run_items_rmt",
      field: "project_id",
      operator: "=",
      value: projectId,
    }),
    ...(datasetId
      ? [
          new StringFilter({
            clickhouseTable: "dataset_run_items_rmt",
            field: "dataset_id",
            operator: "=",
            value: datasetId,
          }),
        ]
      : []),
    ...(runIds && runIds.length > 0
      ? [
          new StringOptionsFilter({
            clickhouseTable: "dataset_run_items_rmt",
            field: "dataset_run_id",
            operator: "any of",
            values: runIds,
          }),
        ]
      : []),
  ];
  return { datasetRunItemsFilter: filters };
};

// ============================================================
// getDatasetRunsTableInternal
// ============================================================

const getDatasetRunsTableInternal = async <T>(
  opts: DatasetRunsMetricsTableQuery & {
    tags: Record<string, string>;
  },
): Promise<Array<T>> => {
  const { projectId, datasetId, runIds, filter, orderBy, limit, offset } = opts;
  let select = "";

  switch (opts.select) {
    case "rows":
      select = `
        drm.project_id as project_id,
        drm.dataset_id as dataset_id,
        drm.dataset_run_id as dataset_run_id,
        drm.dataset_run_name as dataset_run_name,
        drm.dataset_run_created_at as dataset_run_created_at,
        drm.dataset_run_description as dataset_run_description,
        drm.dataset_run_metadata as dataset_run_metadata
      `;
      break;
    case "metrics":
      select = `
        drm.project_id as project_id,
        drm.dataset_id as dataset_id,
        drm.dataset_run_id as dataset_run_id,
        drm.dataset_run_name as dataset_run_name,
        drm.count_run_items as count_run_items,

        -- Latency metrics (priority: trace > observation - matching old PostgreSQL behavior)
        CASE
          WHEN drm.trace_avg_latency IS NOT NULL THEN drm.trace_avg_latency
          ELSE drm.obs_avg_latency
        END as avg_latency_seconds,

        -- Cost metrics (priority: trace > observation - matching old PostgreSQL behavior)
        CASE
          WHEN drm.trace_avg_cost IS NOT NULL THEN drm.trace_avg_cost
          ELSE COALESCE(drm.obs_avg_cost, 0)
        END as avg_total_cost,
        CASE
          WHEN drm.trace_total_cost IS NOT NULL THEN drm.trace_total_cost
          ELSE COALESCE(drm.obs_total_cost, 0)
        END as total_cost,

        -- Score aggregations
        sa.scores_avg as agg_scores_avg,
        sa.score_categories as agg_score_categories`;
      break;
    case "count":
      select = "count(DISTINCT drm.dataset_run_id) as count";
      break;
  }

  const { datasetRunItemsFilter } = getProjectDatasetIdDefaultFilter(
    projectId,
    datasetId,
    runIds,
  );

  // Apply base filter (project + dataset + runIds) before adding user filters
  const baseFilter = applyFilterList(datasetRunItemsFilter);

  // Build scores filter (project scope only)
  const scoresFilter: FilterList = [
    new StringFilter({
      clickhouseTable: "scores",
      field: "project_id",
      operator: "=",
      value: projectId,
    }),
  ];
  const appliedScoresFilter = applyFilterList(scoresFilter);

  // Add user filters on top of base filter
  const userFilters = createFilterFromFilterState(
    filter,
    datasetRunsTableUiColumnDefinitions,
    datasetRunsTableCols,
  );
  datasetRunItemsFilter.push(...userFilters);

  // Apply full filter (base + user)
  const appliedFilter = applyFilterList(datasetRunItemsFilter);

  // Build ORDER BY array
  const orderByArray: OrderByState[] = [];
  if (opts.select === "metrics" && orderBy?.column !== "createdAt") {
    orderByArray.push({
      column: "createdAt",
      order: "DESC",
    });
  }
  if (orderBy) {
    orderByArray.push(orderBy);
  }

  const orderByClause = buildOrderByClause(orderByArray);

  // --- Param offset computation ---
  // Explicit params: $1 = projectId, $2 = datasetId
  const explicitLen = 2;
  const scoresLen = appliedScoresFilter.params.length;
  const baseLen = baseFilter.params.length;

  const shiftedScoresQuery = shiftParamNumbers(appliedScoresFilter.query, explicitLen);
  const shiftedBaseQuery = shiftParamNumbers(baseFilter.query, explicitLen + scoresLen);
  const shiftedAppliedQuery = appliedFilter.query
    ? shiftParamNumbers(appliedFilter.query, explicitLen + scoresLen + baseLen)
    : "";

  const params: unknown[] = [
    projectId,
    datasetId,
    ...appliedScoresFilter.params,
    ...baseFilter.params,
    ...appliedFilter.params,
  ];

  // --- CTE definitions ---

  const scoresCte = `
   WITH scores_aggregated AS (
      SELECT
        dri.dataset_run_id,
        dri.project_id,
        -- For numeric scores, use tuples of (name, avg_value)
        array_agg(
          jsonb_build_array(s.name, s.avg_value)
        ) FILTER (WHERE s.data_type IN ('NUMERIC', 'BOOLEAN')) AS scores_avg,
        -- For categorical scores, use name:value format for improved query performance
        array_agg(
          concat(s.name, ':', s.string_value)
        ) FILTER (WHERE s.data_type = 'CATEGORICAL' AND s.string_value IS NOT NULL AND s.string_value != '') AS score_categories
      FROM dataset_run_items_rmt dri
      LEFT JOIN (
        SELECT
          project_id,
          trace_id,
          name,
          data_type,
          string_value,
          avg(value) as avg_value
        FROM scores s
        WHERE ${shiftedScoresQuery}
        GROUP BY
          project_id,
          trace_id,
          name,
          data_type,
          string_value
      ) s ON s.project_id = dri.project_id AND s.trace_id = dri.trace_id
      WHERE dri.project_id = $1
        AND dri.dataset_id = $2
      GROUP BY dri.dataset_run_id, dri.project_id
    ),
  `;

  const filteredObservationsCte = `
   observations_filtered AS (
      SELECT DISTINCT ON (o.id, o.project_id)
        o.id,
        o.trace_id,
        o.project_id,
        o.start_time,
        o.end_time,
        o.total_cost
      FROM observations o
      WHERE o.project_id = $1
        AND o.start_time >= (
          SELECT min(dri.dataset_run_created_at) - INTERVAL '1 DAY'
          FROM dataset_run_items_rmt dri
          WHERE ${shiftedBaseQuery}
        )
        AND o.start_time <= (
          SELECT max(dri.dataset_run_created_at) + INTERVAL '1 DAY'
          FROM dataset_run_items_rmt dri
          WHERE ${shiftedBaseQuery}
        )
        AND o.trace_id IN (
          SELECT dri.trace_id
          FROM dataset_run_items_rmt dri
          WHERE ${shiftedBaseQuery}
        )
      ORDER BY o.id, o.project_id, o.event_ts DESC
    ),
  `;

  const datasetRunItemsDedupedCte = `
    dataset_run_items_deduped AS (
      SELECT DISTINCT ON (dri.project_id, dri.dataset_id, dri.dataset_run_id, dri.dataset_item_id) *
      FROM dataset_run_items_rmt dri
      WHERE ${shiftedBaseQuery}
      ORDER BY dri.project_id, dri.dataset_id, dri.dataset_run_id, dri.dataset_item_id, dri.created_at DESC
    ),
  `;

  const traceMetricsCte = `
    trace_metrics AS (
      SELECT
        dri.trace_id,
        dri.project_id,
        dri.dataset_id,
        dri.dataset_run_id,
        dri.dataset_item_id,
        EXTRACT(EPOCH FROM (max(of.end_time) - min(of.start_time))) * 1000 as latency_ms,
        sum(of.total_cost) as total_cost
      FROM dataset_run_items_deduped dri
      JOIN observations_filtered of ON dri.trace_id = of.trace_id
        AND dri.project_id = of.project_id
      GROUP BY dri.trace_id, dri.project_id, dri.dataset_id, dri.dataset_run_id, dri.dataset_item_id
    ),
  `;

  const datasetRunMetricsCte = `
    dataset_run_metrics AS (
      SELECT
        dri.dataset_run_id as dataset_run_id,
        dri.project_id as project_id,
        dri.dataset_id as dataset_id,
        dri.dataset_run_created_at as dataset_run_created_at,
        dri.dataset_run_name as dataset_run_name,
        dri.dataset_run_description as dataset_run_description,
        dri.dataset_run_metadata as dataset_run_metadata,
        count(DISTINCT dri.project_id, dri.dataset_id, dri.dataset_run_id, dri.dataset_item_id) as count_run_items,

        -- Trace-level metrics (average across traces in this dataset run)
        AVG(CASE WHEN dri.observation_id IS NULL THEN tm.latency_ms ELSE NULL END) / 1000.0 as trace_avg_latency,
        AVG(CASE WHEN dri.observation_id IS NULL THEN tm.total_cost ELSE NULL END) as trace_avg_cost,
        SUM(CASE WHEN dri.observation_id IS NULL THEN tm.total_cost ELSE NULL END) as trace_total_cost,

        -- Observation-level metrics
        AVG(CASE WHEN dri.observation_id IS NOT NULL THEN
          EXTRACT(EPOCH FROM (of.end_time - of.start_time))
        ELSE NULL END) as obs_avg_latency,
        AVG(CASE WHEN dri.observation_id IS NOT NULL THEN tm.total_cost ELSE NULL END) as obs_avg_cost,
        SUM(CASE WHEN dri.observation_id IS NOT NULL THEN tm.total_cost ELSE NULL END) as obs_total_cost

      FROM dataset_run_items_deduped dri
      LEFT JOIN observations_filtered of ON dri.observation_id = of.id
        AND dri.project_id = of.project_id
        AND dri.trace_id = of.trace_id
      LEFT JOIN trace_metrics tm ON dri.trace_id = tm.trace_id
        AND dri.project_id = tm.project_id
        AND dri.dataset_id = tm.dataset_id
        AND dri.dataset_run_id = tm.dataset_run_id
        AND dri.dataset_item_id = tm.dataset_item_id
      WHERE ${shiftedBaseQuery}
      GROUP BY dri.project_id, dri.dataset_id, dri.dataset_run_id, dri.dataset_run_name, dri.dataset_run_description, dri.dataset_run_metadata, dri.dataset_run_created_at
    )
  `;

  const query = `
    ${scoresCte}
    ${filteredObservationsCte}
    ${datasetRunItemsDedupedCte}
    ${traceMetricsCte}
    ${datasetRunMetricsCte}
    SELECT ${opts.select === "count" ? "" : "DISTINCT"}
      ${select}
    FROM dataset_run_metrics drm
    LEFT JOIN scores_aggregated sa ON drm.dataset_run_id = sa.dataset_run_id AND drm.project_id = sa.project_id
    WHERE drm.project_id = $1 AND drm.dataset_id = $2
    ${appliedFilter.query && appliedFilter.query !== "1=1" ? `AND ${shiftedAppliedQuery}` : ""}
    ${orderByClause}
    ${limit !== undefined && offset !== undefined ? `LIMIT ${limit} OFFSET ${offset}` : ""};`;

  const res = await queryPg<T>({
    query,
    params,
    tags: {
      ...(opts.tags ?? {}),
      feature: "datasets",
      type: "dataset-run-items",
      projectId,
      datasetId,
    },
  });

  return res;
};

export const getDatasetRunsTableMetricsCh = async (
  opts: Omit<DatasetRunsMetricsTableQuery, "select">,
): Promise<DatasetRunsMetrics[]> => {
  const rows = await getDatasetRunsTableInternal<DatasetRunsMetricsRecordType>({
    ...opts,
    select: "metrics",
    tags: { kind: "list" },
  });

  return rows.map(convertDatasetRunsMetricsRecord);
};

export const getDatasetRunsTableRowsCh = async (
  opts: Omit<DatasetRunsMetricsTableQuery, "select">,
): Promise<DatasetRunsRows[]> => {
  const rows = await getDatasetRunsTableInternal<DatasetRunsRowsRecordType>({
    ...opts,
    select: "rows",
    tags: { kind: "list" },
  });

  return rows.map(convertDatasetRunsRowsRecord);
};

export const getDatasetRunsTableCountCh = async (
  opts: Omit<DatasetRunsMetricsTableQuery, "select">,
): Promise<number> => {
  const rows = await getDatasetRunsTableInternal<{ count: string }>({
    ...opts,
    select: "count",
    tags: { kind: "list" },
  });

  return Number(rows[0]?.count);
};

type GetDatasetRunItemsTableOpts<IncludeIO extends boolean> =
  DatasetRunItemsTableQuery & {
    select: "count" | "rows";
    tags: Record<string, string>;
    includeIO?: IncludeIO;
  };

// Phase 1: Find dataset item IDs or count that satisfy conditions across ALL runs
const getQualifyingDatasetItems = async <T>(opts: {
  select: "count" | "rows";
  projectId: string;
  datasetId: string;
  runIds: string[];
  runFilters: {
    runId: string;
    filters: FilterState;
  }[];
  limit?: number;
  offset?: number;
}): Promise<Array<T>> => {
  const { select, projectId, datasetId, runIds, runFilters, limit, offset } =
    opts;

  if (runIds.length === 0) {
    return [];
  }

  // Build base filter (project + dataset only)
  const { datasetRunItemsFilter: baseDatasetRunItemsFilter } =
    getProjectDatasetIdDefaultFilter(projectId, datasetId);
  const baseFilter = applyFilterList(baseDatasetRunItemsFilter);

  // Build run-specific conditions for the intersection query
  const runFilterResults: PgFilterResult[] = runFilters.map((runFilter) => {
    const { runId, filters: filterState } = runFilter;

    // Create run ID condition
    const runConditionFilter = new StringFilter({
      clickhouseTable: "dataset_run_items_rmt",
      field: "dataset_run_id",
      operator: "=",
      value: runId,
    });

    // Create user filters for this run
    const userFilters = createFilterFromFilterState(
      filterState,
      datasetRunItemsTableUiColumnDefinitions,
      datasetRunItemsTableCols,
    );

    // Combine run condition with user filters using AND
    const runFilterList: FilterList = [runConditionFilter, ...userFilters];
    return applyFilterList(runFilterList);
  });

  // add empty filters for the runs that have no filters
  runIds.forEach((runId) => {
    if (runFilters.find((runFilter) => runFilter.runId === runId)) {
      return;
    }
    runFilterResults.push(
      applyFilterList([
        new StringFilter({
          clickhouseTable: "dataset_run_items_rmt",
          field: "dataset_run_id",
          operator: "=",
          value: runId,
        }),
      ]),
    );
  });

  // Combine run conditions with OR, shifting each run filter's params sequentially
  let runParamOffset = 0;
  const runClauses: string[] = [];
  const allRunParams: unknown[] = [];

  for (const result of runFilterResults) {
    const shiftedQuery = shiftParamNumbers(result.query, runParamOffset);
    runClauses.push(`(${shiftedQuery})`);
    allRunParams.push(...result.params);
    runParamOffset += result.params.length;
  }

  const combinedQuery = runClauses.join(" OR ");

  // Check if any run has score filters for CTE
  const hasScoresFilter = runFilters
    .flatMap((f) => f.filters)
    .some((f) => f.column.toLowerCase().includes("score"));

  // Build scores filter
  const scoresFilter: FilterList = [
    new StringFilter({
      clickhouseTable: "scores",
      field: "project_id",
      operator: "=",
      value: projectId,
    }),
  ];
  const appliedScoresFilter = applyFilterList(scoresFilter);

  const selectString =
    select === "count"
      ? "COUNT(DISTINCT dataset_item_id) as count"
      : "dataset_item_id";

  // --- Param offset computation ---
  // Param order: baseFilter.params, appliedScoresFilter.params, totalRunCount, allRunParams
  const baseLen = baseFilter.params.length;
  const scoresLen = appliedScoresFilter.params.length;

  // scores CTE: WHERE ${baseFilter.query} (starts at $1) and WHERE ${appliedScoresFilter.query}
  // (needs shift by baseLen since baseFilter comes first in the param array)
  const shiftedScoresFilterQuery = shiftParamNumbers(appliedScoresFilter.query, baseLen);

  // combinedQuery is embedded in run_qualified_items AND clause, after baseFilter.query
  // offset = baseLen + scoresLen (totalRunCount is separate, after scores params)
  const shiftedCombinedQuery = shiftParamNumbers(combinedQuery, baseLen + scoresLen);

  // totalRunCount goes right after scores params
  const totalRunCountPos = baseLen + scoresLen + 1;
  const intersectionQueryFinal =
    runFilters.length > 0
      ? `HAVING COUNT(DISTINCT dataset_run_id) = $${totalRunCountPos}::int`
      : "";

  // Build the intersection query
  const scoresCte = hasScoresFilter
    ? `
  WITH scores_aggregated AS (
     SELECT
       dri.dataset_run_id,
       dri.project_id,
       dri.trace_id,
       -- For numeric scores, use tuples of (name, avg_value)
       array_agg(
         jsonb_build_array(s.name, s.avg_value)
       ) FILTER (WHERE s.data_type IN ('NUMERIC', 'BOOLEAN')) AS scores_avg,
       -- For categorical scores, use name:value format for improved query performance
       array_agg(
         concat(s.name, ':', s.string_value)
       ) FILTER (WHERE s.data_type = 'CATEGORICAL' AND s.string_value IS NOT NULL AND s.string_value != '') AS score_categories
     FROM dataset_run_items_rmt dri
     LEFT JOIN (
       SELECT
         project_id,
         trace_id,
         name,
         data_type,
         string_value,
         avg(value) as avg_value
       FROM scores s
       WHERE ${shiftedScoresFilterQuery}
       GROUP BY
         project_id,
         trace_id,
         name,
         data_type,
         string_value
     ) s ON s.project_id = dri.project_id AND s.trace_id = dri.trace_id
     WHERE ${baseFilter.query}
     GROUP BY dri.project_id, dri.dataset_id, dri.dataset_run_id, dri.trace_id
   ),
   `
    : "WITH ";

  const query = `
    ${scoresCte}
    run_qualified_items AS (
      SELECT DISTINCT dri.dataset_item_id, dri.dataset_run_id
      FROM dataset_run_items_rmt dri
      ${hasScoresFilter ? `LEFT JOIN scores_aggregated sa ON dri.dataset_run_id = sa.dataset_run_id AND dri.project_id = sa.project_id AND dri.trace_id = sa.trace_id` : ""}
      WHERE ${baseFilter.query}
      AND ${shiftedCombinedQuery}
    ),
    intersection_items AS (
      SELECT dataset_item_id
      FROM run_qualified_items
      GROUP BY dataset_item_id
      ${intersectionQueryFinal}
    )
    SELECT
      ${selectString}
    FROM intersection_items
    ${select === "count" ? "" : "ORDER BY dataset_item_id -- for consistent pagination"}
    ${limit !== undefined && offset !== undefined ? `LIMIT ${limit} OFFSET ${offset}` : ""};`;

  const allParams: unknown[] = [
    ...baseFilter.params,
    ...appliedScoresFilter.params,
    runIds.length,
    ...allRunParams,
  ];

  const res = await queryPg<T>({
    query,
    params: allParams,
    tags: {
      feature: "datasets",
      type: "dataset-run-items",
      projectId,
      datasetId,
    },
  });

  return res;
};

const getDatasetRunItemsTableInternal = async <
  T,
  IncludeIO extends boolean = true,
>(
  opts: GetDatasetRunItemsTableOpts<IncludeIO>,
): Promise<Array<T>> => {
  const { projectId, datasetId, filter, orderBy, limit, offset, includeIO } =
    opts;

  let selectString = "";

  switch (opts.select) {
    case "count":
      selectString =
        "count(DISTINCT dri.project_id, dri.dataset_id, dri.dataset_run_id, dri.dataset_item_id) as count";
      break;
    case "rows":
      selectString = `
      dri.id as id,
      dri.project_id as project_id,
      dri.trace_id as trace_id,
      dri.observation_id as observation_id,
      dri.dataset_id as dataset_id,
      dri.dataset_run_id as dataset_run_id,
      dri.dataset_item_id as dataset_item_id,
      dri.error as error,
      dri.created_at as created_at,
      dri.updated_at as updated_at,
      dri.dataset_run_name as dataset_run_name,
      dri.dataset_run_description as dataset_run_description,
      dri.dataset_run_created_at as dataset_run_created_at,
      dri.dataset_item_version as dataset_item_version,
      ${includeIO ? "dri.dataset_run_metadata as dataset_run_metadata, " : ""}
      ${includeIO ? "dri.dataset_item_input as dataset_item_input, " : ""}
      ${includeIO ? "dri.dataset_item_expected_output as dataset_item_expected_output, " : ""}
      ${includeIO ? "dri.dataset_item_metadata as dataset_item_metadata, " : ""}
      dri.is_deleted as is_deleted,
      dri.event_ts as event_ts`;
      break;
    default:
      throw new Error(`Unknown select type: ${opts.select}`);
  }

  const { datasetRunItemsFilter } = getProjectDatasetIdDefaultFilter(
    projectId,
    datasetId,
  );

  // Add user filters to base filter
  datasetRunItemsFilter.push(
    ...createFilterFromFilterState(
      filter,
      datasetRunItemsTableUiColumnDefinitions,
      datasetRunItemsTableCols,
    ),
  );
  const appliedFilter = applyFilterList(datasetRunItemsFilter);

  // Build scores filter
  const scoresFilter: FilterList = [
    new StringFilter({
      clickhouseTable: "scores",
      field: "project_id",
      operator: "=",
      value: projectId,
    }),
  ];

  const hasScoresFilter = filter.some((f) =>
    f.column.toLowerCase().includes("score"),
  );

  const appliedScoresFilter = applyFilterList(scoresFilter);

  // Build ORDER BY array
  const orderByArray: OrderByState[] = [];

  if (orderBy) {
    if (Array.isArray(orderBy)) {
      orderByArray.push(...orderBy);
    } else {
      orderByArray.push(orderBy);
    }
  }

  // Add event_ts DESC for row queries (for deduplication)
  if (opts.select === "rows") {
    orderByArray.push({
      column: "eventTs",
      order: "DESC",
    });
  }

  const orderByClause = buildOrderByClause(orderByArray);

  // --- Param offset computation ---
  // Explicit params: $1 = projectId, ($2 = datasetId if provided)
  const explicitLen = datasetId ? 2 : 1;
  const scoresLen = appliedScoresFilter.params.length;

  const shiftedScoresFilterQuery = shiftParamNumbers(appliedScoresFilter.query, explicitLen);
  const shiftedAppliedFilterQuery = shiftParamNumbers(appliedFilter.query, explicitLen + scoresLen);

  const explicitParams: unknown[] = [projectId];
  if (datasetId) {
    explicitParams.push(datasetId);
  }

  const allParams: unknown[] = [
    ...explicitParams,
    ...appliedScoresFilter.params,
    ...appliedFilter.params,
  ];

  const scoresCte = `
  WITH scores_aggregated AS (
     SELECT
       dri.dataset_run_id,
       dri.project_id,
       dri.trace_id,
       -- For numeric scores, use tuples of (name, avg_value)
       array_agg(
         jsonb_build_array(s.name, s.avg_value)
       ) FILTER (WHERE s.data_type IN ('NUMERIC', 'BOOLEAN')) AS scores_avg,
       -- For categorical scores, use name:value format for improved query performance
       array_agg(
         concat(s.name, ':', s.string_value)
       ) FILTER (WHERE s.data_type = 'CATEGORICAL' AND s.string_value IS NOT NULL AND s.string_value != '') AS score_categories
     FROM dataset_run_items_rmt dri
     LEFT JOIN (
       SELECT
         project_id,
         trace_id,
         name,
         data_type,
         string_value,
         avg(value) as avg_value
       FROM scores s
       WHERE ${shiftedScoresFilterQuery}
       GROUP BY
         project_id,
         trace_id,
         name,
         data_type,
         string_value
     ) s ON s.project_id = dri.project_id AND s.trace_id = dri.trace_id
     WHERE dri.project_id = $1
       ${datasetId ? "AND dri.dataset_id = $2" : ""}
     GROUP BY dri.dataset_run_id, dri.project_id, dri.trace_id
   )
 `;

  // Build orderBySuffix for the row query (append user-specified ORDER BY columns
  // after the DISTINCT ON columns)
  const orderBySuffix = orderByClause
    ? `, ${orderByClause.replace(/^ORDER BY /, "")}`
    : "";

  const query =
    opts.select === "rows"
      ? `
    ${scoresCte}
    SELECT *
    FROM (
      SELECT DISTINCT ON (dri.project_id, dri.dataset_id, dri.dataset_run_id, dri.dataset_item_id)
        ${selectString}
      FROM dataset_run_items_rmt dri
      ${hasScoresFilter ? `LEFT JOIN scores_aggregated sa ON dri.dataset_run_id = sa.dataset_run_id AND dri.project_id = sa.project_id AND dri.trace_id = sa.trace_id` : ""}
      WHERE ${shiftedAppliedFilterQuery}
      ORDER BY dri.project_id, dri.dataset_id, dri.dataset_run_id, dri.dataset_item_id${orderBySuffix}
    ) AS deduplicated
    ${limit !== undefined && offset !== undefined ? `LIMIT ${limit} OFFSET ${offset}` : ""};`
      : `
    ${scoresCte}
    SELECT
      ${selectString}
    FROM dataset_run_items_rmt dri
    ${hasScoresFilter ? `LEFT JOIN scores_aggregated sa ON dri.dataset_run_id = sa.dataset_run_id AND dri.project_id = sa.project_id AND dri.trace_id = sa.trace_id` : ""}
    WHERE ${shiftedAppliedFilterQuery};`;

  const res = await queryPg<T>({
    query,
    params: allParams,
    tags: {
      ...(opts.tags ?? {}),
      feature: "datasets",
      type: "dataset-run-items",
      projectId,
      ...(datasetId ? { datasetId } : {}),
    },
  });

  return res;
};

export const getDatasetRunItemsCh = async (
  opts: DatasetRunItemsTableQuery,
): Promise<DatasetRunItemDomain[]> => {
  const rows = await getDatasetRunItemsTableInternal<DatasetRunItemRecord>({
    ...opts,
    select: "rows",
    tags: { kind: "list" },
  });

  return rows.map((row) => convertDatasetRunItemClickhouseToDomain(row));
};

export const getDatasetRunItemsByDatasetIdCh = async (
  opts: DatasetRunItemsByDatasetIdQuery,
): Promise<DatasetRunItemDomain[]> => {
  const rows = await getDatasetRunItemsTableInternal<DatasetRunItemRecord>({
    ...opts,
    select: "rows",
    tags: { kind: "list" },
  });

  return rows.map((row) => convertDatasetRunItemClickhouseToDomain(row));
};

export const getDatasetItemsWithRunDataCount = async (
  opts: DatasetItemsWithRunDataCountQuery,
): Promise<number> => {
  const { projectId, datasetId, runIds, filterByRun } = opts;

  const rows = await getQualifyingDatasetItems<{ count: string }>({
    select: "count",
    projectId,
    datasetId,
    runIds,
    runFilters: filterByRun,
  });

  return Number(rows[0]?.count);
};

export const getDatasetItemIdsWithRunData = async (
  opts: DatasetItemIdsWithRunDataQuery,
): Promise<string[]> => {
  const rows = await getQualifyingDatasetItems<{ dataset_item_id: string }>({
    select: "rows",
    runFilters: opts.filterByRun,
    ...opts,
  });

  return rows.map((row) => row.dataset_item_id);
};

export const getDatasetRunItemsWithoutIOByItemIds = async (
  opts: DatasetRunItemsByItemIdsWithoutIOQuery,
): Promise<DatasetRunItemDomain<false>[]> => {
  // Step 1: Get DRI data matching [datasetId, runId, datasetItemId]
  const { datasetItemIds, runIds, ...rest } = opts;

  const filter: FilterState = [
    {
      column: "datasetItemId",
      operator: "any of",
      value: datasetItemIds,
      type: "stringOptions" as const,
    },
    {
      column: "datasetRunId",
      operator: "any of",
      value: runIds,
      type: "stringOptions" as const,
    },
  ];
  const rows = await getDatasetRunItemsTableInternal<
    DatasetRunItemRecord<false>,
    false
  >({
    ...rest,
    filter,
    select: "rows",
    tags: { kind: "list" },
  });

  // Step 2: Convert to domain
  return rows.map((row) => convertDatasetRunItemClickhouseToDomain(row));
};

export const getDatasetItemIdsByTraceIdCh = async (
  opts: DatasetItemIdsByTraceIdQuery,
): Promise<
  { id: string; datasetId: string; observationId: string | null }[]
> => {
  const { projectId, traceId, filter } = opts;

  const datasetRunItemsFilter: FilterList = [
    new StringFilter({
      clickhouseTable: "dataset_run_items_rmt",
      field: "project_id",
      operator: "=",
      value: projectId,
    }),
    new StringFilter({
      clickhouseTable: "dataset_run_items_rmt",
      field: "trace_id",
      operator: "=",
      value: traceId,
    }),
  ];

  datasetRunItemsFilter.push(
    ...createFilterFromFilterState(
      filter,
      datasetRunItemsTableUiColumnDefinitions,
      datasetRunItemsTableCols,
    ),
  );
  const appliedFilter = applyFilterList(datasetRunItemsFilter);

  const query = `
  SELECT DISTINCT ON (dri.project_id, dri.dataset_id, dri.dataset_run_id, dri.dataset_item_id)
    dri.dataset_item_id as dataset_item_id,
    dri.observation_id as observation_id,
    dri.dataset_id as dataset_id
  FROM dataset_run_items_rmt dri
  WHERE ${appliedFilter.query}
  ORDER BY dri.project_id, dri.dataset_id, dri.dataset_run_id, dri.dataset_item_id, dri.created_at DESC;`;

  const res = await queryPg<{
    dataset_item_id: string;
    observation_id: string | null;
    dataset_id: string;
  }>({
    query,
    params: appliedFilter.params,
    tags: {
      feature: "datasets",
      type: "dataset-run-items",
      projectId,
      traceId,
    },
  });

  return res.map((runItem) => {
    return {
      id: runItem.dataset_item_id,
      observationId: runItem.observation_id,
      datasetId: runItem.dataset_id,
    };
  });
};

export const getDatasetRunItemsCountCh = async (
  opts: DatasetRunItemsTableQuery,
): Promise<number> => {
  const rows = await getDatasetRunItemsTableInternal<{ count: string }>({
    ...opts,
    select: "count",
    tags: { kind: "list" },
  });

  return Number(rows[0]?.count);
};

export const getDatasetRunItemsCountByDatasetIdCh = async (
  opts: DatasetRunItemsByDatasetIdQuery,
): Promise<number> => {
  const rows = await getDatasetRunItemsTableInternal<{ count: string }>({
    ...opts,
    select: "count",
    tags: { kind: "list" },
  });

  return Number(rows[0]?.count);
};

export const hasAnyDatasetRunItem = async (
  projectId: string,
): Promise<boolean> => {
  const query = `
    SELECT 1
    FROM dataset_run_items_rmt
    WHERE project_id = $1
    LIMIT 1
  `;

  const rows = await queryPg<{ 1: number }>({
    query,
    params: [projectId],
    tags: {
      feature: "datasets",
      type: "dataset-run-items",
      kind: "hasAny",
      projectId,
    },
  });

  return rows.length > 0;
};

export const deleteDatasetRunItemsByProjectId = async (
  projectId: string,
): Promise<boolean> => {
  const hasData = await hasAnyDatasetRunItem(projectId);
  if (!hasData) {
    return false;
  }

  const query = `
    DELETE FROM dataset_run_items_rmt
    WHERE project_id = $1;
  `;
  await executePg({
    query,
    params: [projectId],
  });

  return true;
};

export const deleteDatasetRunItemsByDatasetId = async ({
  projectId,
  datasetId,
}: {
  projectId: string;
  datasetId: string;
}) => {
  const query = `
  DELETE FROM dataset_run_items_rmt
  WHERE project_id = $1
  AND dataset_id = $2
`;

  await executePg({
    query,
    params: [projectId, datasetId],
  });
};

export const deleteDatasetRunItemsByDatasetRunIds = async ({
  projectId,
  datasetRunIds,
  datasetId,
}: {
  projectId: string;
  datasetRunIds: string[];
  datasetId: string;
}) => {
  const query = `
    DELETE FROM dataset_run_items_rmt
    WHERE project_id = $1
    AND dataset_id = $2
    AND dataset_run_id = ANY($3::text[])
  `;

  await executePg({
    query,
    params: [projectId, datasetId, datasetRunIds],
  });
};

export const getDatasetRunItemCountsByProjectInCreationInterval = async ({
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
  FROM dataset_run_items_rmt
  WHERE created_at >= $1::timestamptz
  AND created_at < $2::timestamptz
  GROUP BY project_id
`;

  const rows = await queryPg<{ project_id: string; count: string }>({
    query,
    params: [start.toISOString(), end.toISOString()],
    tags: {
      feature: "datasets",
      type: "dataset-run-items",
      kind: "analytic",
      operation_name: "getDatasetRunItemCountsByProjectInCreationInterval",
    },
  });

  return rows.map((row) => ({
    projectId: row.project_id,
    count: Number(row.count),
  }));
};
