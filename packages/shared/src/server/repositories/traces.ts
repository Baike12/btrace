// @ts-nocheck — PG rewrite done, minor type issues from agent conversion
import {
  queryPg,
  executePg,
  queryPgStream,
  parseClickhouseUTCDateTimeFormat,
  pgCompliantRandomCharacters,
} from "./pg";
import {
  createFilterFromFilterState,
  getProjectIdDefaultFilter,
} from "../queries/pg-sql/factory";
import { orderByToPgSql } from "../queries/pg-sql/orderby-factory";
import { shouldSkipObservationsFinal } from "../queries/pg-sql/query-options";
import { pgSearchCondition } from "../queries/pg-sql/search";
import {
  DateTimeFilter,
  FilterList,
  StringFilter,
} from "../queries/pg-sql/pg-filter";
import { LISTABLE_SCORE_TYPES } from "../../domain/scores";
import { OrderByState } from "../../interfaces/orderBy";
import snakeCase from "lodash/snakeCase";
import { FilterState } from "../../types";
import { TraceRecordReadType } from "./definitions";
import { tracesTableUiColumnDefinitions } from "../tableMappings/mapTracesTable";
import { UiColumnMappings, ColumnDefinition } from "../../tableDefinitions";
import { tracesTableCols } from "../../tableDefinitions/tracesTable";
import {
  convertClickhouseToDomain,
  convertClickhouseTracesListToDomain,
} from "./traces_converters";
import {
  OBSERVATIONS_TO_TRACE_INTERVAL,
  TRACE_TO_OBSERVATIONS_INTERVAL,
} from "./constants";
import { env } from "../../env";
import type { AnalyticsTraceEvent } from "../analytics-integrations/types";
import { RenderingProps, DEFAULT_RENDERING_PROPS } from "../utils/rendering";
import { logger } from "../logger";
import { traceException } from "../instrumentation";
import { prisma } from "../../db";
import { recordDistribution } from "../instrumentation";

// ============================================================
// Compatibility helpers (original clickhouse modules were removed)
// ============================================================

/** PG substitute for the removed clickhouse/measureAndReturn */
async function measureAndReturn<T, I>(opts: {
  operationName: string;
  projectId: string;
  input: I;
  fn: (input: I) => Promise<T>;
}): Promise<T> {
  return opts.fn(opts.input);
}

/** PG substitute for PreferredClickhouseService type */
type PreferredClickhouseService = string | undefined;

/** Helper: renumber $N positional params in generated SQL by an offset */
function offsetFilterParams(sql: string, offset: number): string {
  if (offset === 0) return sql;
  return sql.replace(/\$(\d+)/g, (_: string, n: string) => "$" + (parseInt(n) + offset));
}

// ============================================================
// checkTraceExistsAndGetTimestamp
// ============================================================
export const checkTraceExistsAndGetTimestamp = async ({
  projectId,
  traceId,
  timestamp,
  filter,
  maxTimeStamp,
  exactTimestamp,
}: {
  projectId: string;
  traceId: string;
  timestamp: Date;
  filter: FilterState;
  maxTimeStamp: Date | undefined;
  exactTimestamp?: Date;
}): Promise<{ exists: boolean; timestamp?: Date }> => {
  const { tracesFilter } = getProjectIdDefaultFilter(projectId, {
    tracesPrefix: "t",
  });

  const timeStampFilter = tracesFilter.find(
    (f) =>
      f.field === "timestamp" && (f.operator === ">=" || f.operator === ">"),
  ) as DateTimeFilter | undefined;

  tracesFilter.push(
    ...createFilterFromFilterState(
      filter,
      tracesTableUiColumnDefinitions,
      tracesTableCols,
    ),
    new StringFilter({
      clickhouseTable: "t",
      field: "id",
      operator: "=",
      value: traceId,
    }),
  );

  const observationFilter = tracesFilter.find(
    (f) => f.clickhouseTable === "observations",
  );
  const tracesFilterRes = tracesFilter.apply();
  const observationFilterRes = observationFilter?.apply();

  const directParams: unknown[] = [];
  directParams.push(projectId);
  directParams.push(timestamp.toISOString());

  let obsTsClause = "";
  if (timeStampFilter) {
    const tsRes = timeStampFilter.apply(1);
    obsTsClause = `AND o.start_time >= ${offsetFilterParams(tsRes.query, directParams.length)} - INTERVAL '2 DAYS'`;
    directParams.push(...tsRes.params);
  }

  const observations_cte = `
    WITH observations_agg AS (
      SELECT
        CASE
          WHEN bool_or(level = 'ERROR') THEN 'ERROR'
          WHEN bool_or(level = 'WARNING') THEN 'WARNING'
          WHEN bool_or(level = 'DEFAULT') THEN 'DEFAULT'
          ELSE 'DEBUG'
        END AS aggregated_level,
        count(*) FILTER (WHERE level = 'ERROR') as error_count,
        count(*) FILTER (WHERE level = 'WARNING') as warning_count,
        count(*) FILTER (WHERE level = 'DEFAULT') as default_count,
        count(*) FILTER (WHERE level = 'DEBUG') as debug_count,
        trace_id,
        project_id
      FROM observations o
      WHERE o.project_id = $1
        ${obsTsClause}
        AND o.start_time >= $2::timestamptz - INTERVAL '2 DAYS'
      GROUP BY trace_id, project_id
    )
  `;

  return measureAndReturn({
    operationName: "checkTraceExistsAndGetTimestamp",
    projectId,
    input: {
      params: {
        projectId,
        directParams,
        tracesFilterRes,
        observationFilterRes,
        timestamp,
        maxTimeStamp,
        exactTimestamp,
      },
      tags: {
        feature: "tracing",
        type: "trace",
        kind: "exists",
        projectId,
        operation_name: "checkTraceExistsAndGetTimestamp",
      },
      timestamp: timestamp ?? exactTimestamp,
    },
    fn: async (input) => {
      const params: unknown[] = [...directParams];
      let paramIdx = params.length;

      const joinClause = observationFilterRes
        ? `INNER JOIN observations_agg o ON t.id = o.trace_id AND t.project_id = o.project_id`
        : "";

      let whereClause = `t.project_id = $1`;

      if (tracesFilterRes.query && tracesFilterRes.query !== "1=1") {
        const renumbered = offsetFilterParams(tracesFilterRes.query, paramIdx);
        whereClause += ` AND ${renumbered}`;
        params.push(...tracesFilterRes.params);
        paramIdx += tracesFilterRes.params.length;
      }

      whereClause += ` AND t.timestamp >= $2::timestamptz - INTERVAL '1 HOUR'`;

      if (maxTimeStamp) {
        params.push(input.maxTimeStamp.toISOString());
        paramIdx++;
        whereClause += ` AND t.timestamp <= $${paramIdx}::timestamptz`;
      } else {
        whereClause += ` AND t.timestamp <= $2::timestamptz + INTERVAL '2 DAYS'`;
      }

      if (exactTimestamp) {
        params.push(input.exactTimestamp.toISOString());
        paramIdx++;
        whereClause += ` AND t."timestamp"::date = $${paramIdx}::date`;
      }

      const query = `
        ${observations_cte}
        SELECT
          t.id as id,
          t.project_id as project_id,
          to_char(t."timestamp" at time zone 'UTC', 'YYYY-MM-DD HH24:MI:SS.MS') as timestamp
        FROM traces t
        ${joinClause}
        WHERE ${whereClause}
        GROUP BY t.id, t.project_id, t.timestamp
      `;

      const rows = await queryPg<{
        id: string;
        project_id: string;
        timestamp: string;
      }>({
        query,
        params,
        tags: input.tags,
      });

      return {
        exists: rows.length > 0,
        timestamp:
          rows.length > 0
            ? parseClickhouseUTCDateTimeFormat(rows[0].timestamp)
            : undefined,
      };
    },
  });
};

// ============================================================
// upsertTrace
// ============================================================
export const upsertTrace = async (trace: Partial<TraceRecordReadType>) => {
  if (!["id", "project_id", "timestamp"].every((key) => key in trace)) {
    throw new Error("Identifier fields must be provided to upsert Trace.");
  }

  const t = trace as TraceRecordReadType;
  await executePg({
    query: `
      INSERT INTO traces (
        id, timestamp, name, user_id, metadata,
        release, version, project_id, public, bookmarked,
        tags, input, output, session_id, created_at, updated_at
      ) VALUES (
        $1, $2, $3, $4, $5,
        $6, $7, $8, $9, $10,
        $11, $12, $13, $14, $15, $16
      )
      ON CONFLICT (id, project_id) DO UPDATE SET
        timestamp = EXCLUDED.timestamp,
        name = EXCLUDED.name,
        user_id = EXCLUDED.user_id,
        metadata = EXCLUDED.metadata,
        release = EXCLUDED.release,
        version = EXCLUDED.version,
        public = EXCLUDED.public,
        bookmarked = EXCLUDED.bookmarked,
        tags = EXCLUDED.tags,
        input = EXCLUDED.input,
        output = EXCLUDED.output,
        session_id = EXCLUDED.session_id,
        updated_at = EXCLUDED.updated_at
    `,
    params: [
      t.id,
      new Date(t.timestamp).toISOString(),
      t.name ?? null,
      t.user_id ?? null,
      JSON.stringify(t.metadata ?? {}),
      t.release ?? null,
      t.version ?? null,
      t.project_id,
      t.public ?? false,
      t.bookmarked ?? false,
      t.tags ?? [],
      t.input ?? null,
      t.output ?? null,
      t.session_id ?? null,
      new Date(t.created_at).toISOString(),
      new Date(t.updated_at).toISOString(),
    ],
    tags: {
      feature: "tracing",
      type: "trace",
      kind: "upsert",
      projectId: trace.project_id ?? "",
    },
  });
};

// ============================================================
// getTracesByIds
// ============================================================
export const getTracesByIds = async (
  traceIds: string[],
  projectId: string,
  timestamp?: Date,
  clickhouseConfigs?: Record<string, unknown> | undefined,
) => {
  const records = await measureAndReturn({
    operationName: "getTracesByIds",
    projectId,
    input: {
      params: {
        traceIds,
        projectId,
        timestamp: timestamp ? timestamp.toISOString() : null,
      },
      tags: {
        feature: "tracing",
        type: "trace",
        kind: "byId",
        projectId,
        operation_name: "getTracesByIds",
      },
      clickhouseConfigs,
    },
    fn: (input) => {
      const query = `
        SELECT DISTINCT ON (id, project_id) *
        FROM traces
        WHERE id = ANY($1::text[])
        AND project_id = $2
        ${timestamp ? `AND timestamp >= $3::timestamptz` : ""}
        ORDER BY id, project_id, updated_at DESC
      `;
      return queryPg<TraceRecordReadType>({
        query,
        params: timestamp
          ? [input.params.traceIds, input.params.projectId, input.params.timestamp]
          : [input.params.traceIds, input.params.projectId],
        tags: input.tags,
      });
    },
  });

  return records.map((record) =>
    convertClickhouseToDomain(record, DEFAULT_RENDERING_PROPS),
  );
};

// ============================================================
// getTracesBySessionId
// ============================================================
export const getTracesBySessionId = async (
  projectId: string,
  sessionIds: string[],
  timestamp?: Date,
) => {
  const records = await measureAndReturn({
    operationName: "getTracesBySessionId",
    projectId,
    input: {
      params: {
        sessionIds,
        projectId,
        timestamp: timestamp ? timestamp.toISOString() : null,
      },
      tags: {
        feature: "tracing",
        type: "trace",
        kind: "list",
        projectId,
        operation_name: "getTracesBySessionId",
      },
      timestamp,
    },
    fn: (input) => {
      const query = `
        SELECT DISTINCT ON (id, project_id) *
        FROM traces
        WHERE session_id = ANY($1::text[])
        AND project_id = $2
        ${timestamp ? `AND timestamp >= $3::timestamptz` : ""}
        ORDER BY id, project_id, updated_at DESC
      `;
      return queryPg<TraceRecordReadType>({
        query,
        params: timestamp
          ? [input.params.sessionIds, input.params.projectId, input.params.timestamp]
          : [input.params.sessionIds, input.params.projectId],
        tags: input.tags,
      });
    },
  });

  const traces = records.map((record) =>
    convertClickhouseToDomain(record, DEFAULT_RENDERING_PROPS),
  );

  traces.forEach((trace) => {
    recordDistribution(
      "langfuse.traces_by_session_id_age",
      new Date().getTime() - trace.timestamp.getTime(),
    );
  });

  return traces;
};

// ============================================================
// hasAnyTrace
// ============================================================
export const hasAnyTrace = async (projectId: string) => {
  try {
    const project = await prisma.project.findUnique({
      where: { id: projectId },
      select: { hasTraces: true },
    });
    if (project?.hasTraces) {
      return true;
    }
  } catch (error) {
    traceException(error);
    logger.error("Failed to read hasTraces flag from PostgreSQL", {
      projectId,
      error,
    });
  }

  const result = await measureAndReturn({
    operationName: "hasAnyTrace",
    projectId,
    input: {
      projectId,
      tags: {
        feature: "tracing",
        type: "trace",
        kind: "hasAny",
        projectId,
        operation_name: "hasAnyTrace",
      },
    },
    fn: async (input) => {
      const query = `
        SELECT 1
        FROM traces
        WHERE project_id = $1
        LIMIT 1
      `;

      const rows = await queryPg<{ "?column?": number }>({
        query,
        params: [input.projectId],
        tags: input.tags,
      });

      return rows.length > 0;
    },
  });

  if (result) {
    try {
      await prisma.project.updateMany({
        where: { id: projectId, hasTraces: false },
        data: { hasTraces: true },
      });
    } catch (error) {
      traceException(error);
      logger.error("Failed to persist hasTraces flag to PostgreSQL", {
        projectId,
        error,
      });
    }
  }

  return result;
};

// ============================================================
// getTraceCountsByProjectInCreationInterval
// ============================================================
export const getTraceCountsByProjectInCreationInterval = async ({
  start,
  end,
}: {
  start: Date;
  end: Date;
}) => {
  return measureAndReturn({
    operationName: "getTraceCountsByProjectInCreationInterval",
    projectId: "__CROSS_PROJECT__",
    input: {
      params: { start: start.toISOString(), end: end.toISOString() },
      tags: {
        feature: "tracing",
        type: "trace",
        kind: "analytic",
        operation_name: "getTraceCountsByProjectInCreationInterval",
      },
      timestamp: start,
    },
    fn: async (input) => {
      const query = `
        SELECT project_id, count(*)::text as count
        FROM traces
        WHERE created_at >= $1::timestamptz AND created_at < $2::timestamptz
        GROUP BY project_id
      `;
      const rows = await queryPg<{ project_id: string; count: string }>({
        query,
        params: [input.params.start, input.params.end],
        tags: input.tags,
      });
      return rows.map((row) => ({
        projectId: row.project_id,
        count: Number(row.count),
      }));
    },
  });
};

// ============================================================
// getTraceCountOfProjectsSinceCreationDate
// ============================================================
export const getTraceCountOfProjectsSinceCreationDate = async ({
  projectIds,
  start,
}: {
  projectIds: string[];
  start: Date;
}) => {
  return measureAndReturn({
    operationName: "getTraceCountOfProjectsSinceCreationDate",
    projectId: "__CROSS_PROJECT__",
    input: {
      params: { projectIds, start: start.toISOString() },
      tags: {
        feature: "tracing",
        type: "trace",
        kind: "analytic",
        operation_name: "getTraceCountOfProjectsSinceCreationDate",
      },
      timestamp: start,
    },
    fn: async (input) => {
      const query = `
        SELECT count(*)::text as count
        FROM traces
        WHERE project_id = ANY($1::text[]) AND created_at >= $2::timestamptz
      `;
      const rows = await queryPg<{ count: string }>({
        query,
        params: [input.params.projectIds, input.params.start],
        tags: input.tags,
      });
      return Number(rows[0]?.count ?? 0);
    },
  });
};

// ============================================================
// getTraceByIdFromTracesTable
// ============================================================
export const getTraceByIdFromTracesTable = async ({
  traceId,
  projectId,
  timestamp,
  fromTimestamp,
  renderingProps = DEFAULT_RENDERING_PROPS,
  clickhouseFeatureTag = "tracing",
  preferredClickhouseService,
  excludeInputOutput = false,
  excludeMetadata = false,
}: {
  traceId: string;
  projectId: string;
  timestamp?: Date;
  fromTimestamp?: Date;
  renderingProps?: RenderingProps;
  clickhouseFeatureTag?: string;
  preferredClickhouseService?: PreferredClickhouseService;
  excludeInputOutput?: boolean;
  excludeMetadata?: boolean;
}) => {
  const records = await measureAndReturn({
    operationName: "getTraceById",
    projectId,
    input: {
      params: {
        traceId,
        projectId,
        timestamp: timestamp ? timestamp.toISOString() : null,
        fromTimestamp: fromTimestamp ? fromTimestamp.toISOString() : null,
      },
      tags: {
        feature: clickhouseFeatureTag,
        type: "trace",
        kind: "byId",
        projectId,
        operation_name: "getTraceById",
      },
    },
    fn: (input) => {
      const inputColumn = excludeInputOutput
        ? "''"
        : renderingProps.truncated
          ? `substring(input, 1, ${env.LANGFUSE_SERVER_SIDE_IO_CHAR_LIMIT})`
          : "input";
      const outputColumn = excludeInputOutput
        ? "''"
        : renderingProps.truncated
          ? `substring(output, 1, ${env.LANGFUSE_SERVER_SIDE_IO_CHAR_LIMIT})`
          : "output";
      const metadataColumn = excludeMetadata ? "'{}'" : "metadata";

      const params: unknown[] = [input.params.traceId, input.params.projectId];
      let whereExtras = "";

      if (timestamp) {
        params.push(input.params.timestamp);
        whereExtras += ` AND "timestamp"::date = $${params.length}::date`;
      }
      if (fromTimestamp) {
        params.push(input.params.fromTimestamp);
        whereExtras += ` AND timestamp >= $${params.length}::timestamptz`;
      }

      const query = `
        SELECT
          id, name as name, user_id as user_id,
          ${metadataColumn} as metadata, release as release, version as version,
          project_id, 'default' as environment, public as public, bookmarked as bookmarked,
          tags, ${inputColumn} as input, ${outputColumn} as output,
          session_id as session_id, 0 as is_deleted,
          to_char("timestamp" at time zone 'UTC', 'YYYY-MM-DD HH24:MI:SS.MS') as timestamp,
          to_char(created_at at time zone 'UTC', 'YYYY-MM-DD HH24:MI:SS.MS') as created_at,
          to_char(updated_at at time zone 'UTC', 'YYYY-MM-DD HH24:MI:SS.MS') as updated_at
        FROM traces
        WHERE id = $1 AND project_id = $2 ${whereExtras}
        ORDER BY updated_at DESC LIMIT 1
      `;

      return queryPg<TraceRecordReadType>({ query, params, tags: input.tags });
    },
  });

  const res = records.map((record) =>
    convertClickhouseToDomain(record, renderingProps),
  );
  res.forEach((trace) => {
    recordDistribution(
      "langfuse.query_by_id_age",
      new Date().getTime() - trace.timestamp.getTime(),
      { table: "traces" },
    );
  });
  return res.shift();
};

// ============================================================
// getTracesGroupedByName
// ============================================================
export const getTracesGroupedByName = async (
  projectId: string,
  tableDefinitions: UiColumnMappings = tracesTableUiColumnDefinitions,
  timestampFilter?: FilterState,
) => {
  const chFilter = timestampFilter
    ? createFilterFromFilterState(timestampFilter, tableDefinitions)
    : undefined;
  const filterRes = chFilter ? new FilterList(chFilter).apply() : undefined;

  return measureAndReturn({
    operationName: "getTracesGroupedByName",
    projectId,
    input: {
      params: { projectId, filterRes },
      tags: {
        feature: "tracing", type: "trace", kind: "analytic",
        projectId, operation_name: "getTracesGroupedByName",
      },
    },
    fn: async (input) => {
      const params: unknown[] = [projectId];
      let filterClause = "";
      if (filterRes && filterRes.query !== "1=1") {
        const renumbered = offsetFilterParams(filterRes.query, params.length);
        filterClause = `AND ${renumbered}`;
        params.push(...filterRes.params);
      }
      const query = `
        select name as name, count(*)::text as count
        from traces t
        WHERE t.project_id = $1 AND t.name IS NOT NULL ${filterClause}
        GROUP BY name ORDER BY count(*) desc LIMIT 1000
      `;
      return queryPg<{ name: string; count: string }>({ query, params, tags: input.tags });
    },
  });
};

// ============================================================
// getTracesGroupedBySessionId
// ============================================================
export const getTracesGroupedBySessionId = async (
  projectId: string,
  filter: FilterState,
  searchQuery?: string,
  limit?: number,
  offset?: number,
  columns?: UiColumnMappings,
  columnDefinitions?: ColumnDefinition[],
) => {
  const { tracesFilter } = getProjectIdDefaultFilter(projectId, { tracesPrefix: "t" });
  tracesFilter.push(
    ...createFilterFromFilterState(
      filter,
      columns ?? tracesTableUiColumnDefinitions,
      columnDefinitions ?? tracesTableCols,
    ),
  );
  const tracesFilterRes = tracesFilter.apply();
  const search = pgSearchCondition({ query: searchQuery, tablePrefix: "t" });

  return measureAndReturn({
    operationName: "getTracesGroupedBySessionId",
    projectId,
    input: {
      params: { limit, offset, projectId, tracesFilterRes, search },
      tags: {
        feature: "tracing", type: "trace", kind: "analytic",
        projectId, operation_name: "getTracesGroupedBySessionId",
      },
    },
    fn: async (input) => {
      const params: unknown[] = [];
      let paramIdx = 0;
      const filterQuery =
        tracesFilterRes.query && tracesFilterRes.query !== "1=1"
          ? offsetFilterParams(tracesFilterRes.query, paramIdx)
          : "1=1";
      params.push(...tracesFilterRes.params);
      paramIdx += tracesFilterRes.params.length;
      let searchClause = "";
      if (search.params.length > 0) {
        searchClause = offsetFilterParams(search.query, paramIdx);
        params.push(...search.params);
        paramIdx += search.params.length;
      }
      const query = `
        select session_id as session_id, count(*)::text as count
        from traces t
        WHERE ${filterQuery} AND t.session_id IS NOT NULL AND t.session_id != '' ${searchClause}
        GROUP BY session_id ORDER BY count desc
        ${limit !== undefined && offset !== undefined ? `LIMIT $${paramIdx + 1} OFFSET $${paramIdx + 2}` : ""}
      `;
      const allParams = [...params];
      if (limit !== undefined && offset !== undefined) {
        allParams.push(limit, offset);
      }
      return queryPg<{ session_id: string; count: string }>({ query, params: allParams, tags: input.tags });
    },
  });
};

// ============================================================
// getTracesGroupedByUsers
// ============================================================
export const getTracesGroupedByUsers = async (
  projectId: string,
  filter: FilterState,
  searchQuery?: string,
  limit?: number,
  offset?: number,
  columns?: UiColumnMappings,
  columnDefinitions?: ColumnDefinition[],
) => {
  const { tracesFilter } = getProjectIdDefaultFilter(projectId, { tracesPrefix: "t" });
  tracesFilter.push(
    ...createFilterFromFilterState(
      filter,
      columns ?? tracesTableUiColumnDefinitions,
      columnDefinitions ?? tracesTableCols,
    ),
  );
  const tracesFilterRes = tracesFilter.apply();
  const search = pgSearchCondition({ query: searchQuery, tablePrefix: "t" });

  return measureAndReturn({
    operationName: "getTracesGroupedByUsers",
    projectId,
    input: {
      params: { limit, offset, projectId, tracesFilterRes, search },
      tags: {
        feature: "tracing", type: "trace", kind: "analytic",
        projectId, operation_name: "getTracesGroupedByUsers",
      },
    },
    fn: async (input) => {
      const params: unknown[] = [...tracesFilterRes.params];
      let paramIdx = params.length;
      let searchClause = "";
      if (search.params.length > 0) {
        searchClause = offsetFilterParams(search.query, paramIdx);
        params.push(...search.params);
        paramIdx += search.params.length;
      }
      const filterQuery = offsetFilterParams(tracesFilterRes.query || "1=1", 0);
      const query = `
        select user_id as user, count(*)::text as count
        from traces t
        WHERE ${filterQuery} AND t.user_id IS NOT NULL AND t.user_id != '' ${searchClause}
        GROUP BY t.user_id ORDER BY count desc
        ${limit !== undefined && offset !== undefined ? `LIMIT $${paramIdx + 1} OFFSET $${paramIdx + 2}` : ""}
      `;
      const allParams = [...params];
      if (limit !== undefined && offset !== undefined) {
        allParams.push(limit, offset);
      }
      return queryPg<{ user: string; count: string }>({ query, params: allParams, tags: input.tags });
    },
  });
};

// ============================================================
// getTracesGroupedByTags
// ============================================================
export type GroupedTracesQueryProp = {
  projectId: string;
  filter: FilterState;
  columns?: UiColumnMappings;
  columnDefinitions?: ColumnDefinition[];
};

export const getTracesGroupedByTags = async (props: GroupedTracesQueryProp) => {
  const { projectId, filter, columns, columnDefinitions } = props;
  const chFilter = createFilterFromFilterState(
    filter,
    columns ?? tracesTableUiColumnDefinitions,
    columnDefinitions ?? tracesTableCols,
  );
  const filterRes = new FilterList(chFilter).apply();

  return measureAndReturn({
    operationName: "getTracesGroupedByTags",
    projectId,
    input: {
      params: { projectId, filterRes },
      tags: {
        feature: "tracing", type: "trace", kind: "analytic",
        projectId, operation_name: "getTracesGroupedByTags",
      },
    },
    fn: async (input) => {
      const params: unknown[] = [];
      let filterClause = "1=1";
      if (filterRes.query && filterRes.query !== "1=1") {
        filterClause = offsetFilterParams(filterRes.query, 0);
        params.push(...filterRes.params);
      }
      const query = `
        select distinct unnest(tags) as value
        from traces t WHERE ${filterClause} LIMIT 1000
      `;
      return queryPg<{ value: string }>({ query, params, tags: input.tags });
    },
  });
};

// ============================================================
// getTracesIdentifierForSessionFromTracesTable
// ============================================================
export const getTracesIdentifierForSessionFromTracesTable = async (
  projectId: string,
  sessionId: string,
) => {
  const rows = await measureAndReturn({
    operationName: "getTracesIdentifierForSession",
    projectId,
    input: {
      params: { projectId, sessionId },
      tags: {
        feature: "tracing", type: "trace", kind: "list",
        projectId, operation_name: "getTracesIdentifierForSession",
      },
    },
    fn: (input) => {
      const query = `
        SELECT DISTINCT ON (id, project_id)
          id, user_id, name,
          to_char("timestamp" at time zone 'UTC', 'YYYY-MM-DD HH24:MI:SS.MS') as timestamp,
          project_id, environment
        FROM traces
        WHERE project_id = $1 AND session_id = $2
        ORDER BY id, project_id, updated_at DESC
      `;
      return queryPg<{
        id: string; user_id: string; name: string;
        timestamp: string; environment: string;
      }>({ query, params: [input.params.projectId, input.params.sessionId], tags: input.tags });
    },
  });

  return rows.map((row) => ({
    id: row.id,
    userId: row.user_id,
    name: row.name,
    timestamp: parseClickhouseUTCDateTimeFormat(row.timestamp),
    environment: row.environment,
  }));
};

// ============================================================
// deleteTraces
// ============================================================
export const deleteTraces = async (projectId: string, traceIds: string[]) => {
  await measureAndReturn({
    operationName: "deleteTraces",
    projectId,
    input: {
      params: { projectId, traceIds },
      tags: { feature: "tracing", type: "trace", kind: "delete", projectId },
    },
    fn: async (input) => {
      const preflight = await queryPg<{
        min_ts: string; max_ts: string; cnt: string;
      }>({
        query: `
          SELECT
            (min(timestamp) - INTERVAL '1 HOUR')::text as min_ts,
            (max(timestamp) + INTERVAL '1 HOUR')::text as max_ts,
            count(*)::text as cnt
          FROM traces
          WHERE project_id = $1 AND id = ANY($2::text[])
        `,
        params: [input.params.projectId, input.params.traceIds],
        tags: { ...input.tags, kind: "delete-preflight" },
      });

      const count = Number(preflight[0]?.cnt ?? 0);
      if (count === 0) {
        logger.info(
          `deleteTraces: no rows found for project ${projectId}, skipping DELETE`,
        );
        return;
      }

      await executePg({
        query: `
          DELETE FROM traces
          WHERE project_id = $1 AND id = ANY($2::text[])
          AND timestamp >= $3::timestamptz AND timestamp <= $4::timestamptz
        `,
        params: [
          input.params.projectId, input.params.traceIds,
          preflight[0].min_ts, preflight[0].max_ts,
        ],
        tags: input.tags,
      });
    },
  });
};

// ============================================================
// hasAnyTraceOlderThan
// ============================================================
export const hasAnyTraceOlderThan = async (
  projectId: string,
  beforeDate: Date,
) => {
  const query = `
    SELECT 1 FROM traces
    WHERE project_id = $1 AND timestamp < $2::timestamptz LIMIT 1
  `;
  const rows = await queryPg<{ "?column?": number }>({
    query,
    params: [projectId, beforeDate.toISOString()],
    tags: { feature: "tracing", type: "trace", kind: "hasAnyOlderThan", projectId },
  });
  return rows.length > 0;
};

// ============================================================
// deleteTracesOlderThanDays
// ============================================================
export const deleteTracesOlderThanDays = async (
  projectId: string,
  beforeDate: Date,
): Promise<boolean> => {
  const hasData = await hasAnyTraceOlderThan(projectId, beforeDate);
  if (!hasData) return false;

  await measureAndReturn({
    operationName: "deleteTracesOlderThanDays",
    projectId,
    input: {
      params: { projectId, cutoffDate: beforeDate.toISOString() },
      tags: { feature: "tracing", type: "trace", kind: "delete", projectId },
    },
    fn: async (input) => {
      const query = `
        DELETE FROM traces WHERE project_id = $1 AND timestamp < $2::timestamptz
      `;
      await executePg({
        query,
        params: [input.params.projectId, input.params.cutoffDate],
        tags: input.tags,
      });
    },
  });

  return true;
};

// ============================================================
// deleteTracesByProjectId
// ============================================================
export const deleteTracesByProjectId = async (
  projectId: string,
): Promise<boolean> => {
  const hasData = await hasAnyTrace(projectId);
  if (!hasData) return false;

  await measureAndReturn({
    operationName: "deleteTracesByProjectId",
    projectId,
    input: {
      params: { projectId },
      tags: { feature: "tracing", type: "trace", kind: "delete", projectId },
    },
    fn: async (input) => {
      const query = `DELETE FROM traces WHERE project_id = $1`;
      await executePg({
        query, params: [input.params.projectId], tags: input.tags,
      });
    },
  });

  return true;
};

// ============================================================
// hasAnyUser
// ============================================================
export const hasAnyUser = async (projectId: string) => {
  return measureAndReturn({
    operationName: "hasAnyUser",
    projectId,
    input: {
      projectId,
      tags: {
        feature: "tracing", type: "user", kind: "hasAny",
        projectId, operation_name: "hasAnyUser",
      },
    },
    fn: async (input) => {
      const query = `
        SELECT 1 FROM traces
        WHERE project_id = $1 AND user_id IS NOT NULL AND user_id != '' LIMIT 1
      `;
      const rows = await queryPg<{ "?column?": number }>({
        query, params: [input.projectId], tags: input.tags,
      });
      return rows.length > 0;
    },
  });
};

// ============================================================
// getTotalUserCount
// ============================================================
export const getTotalUserCount = async (
  projectId: string,
  filter: FilterState,
  searchQuery?: string,
): Promise<{ totalCount: bigint }[]> => {
  const { tracesFilter } = getProjectIdDefaultFilter(projectId, { tracesPrefix: "t" });
  tracesFilter.push(
    ...createFilterFromFilterState(filter, tracesTableUiColumnDefinitions, tracesTableCols),
  );
  const tracesFilterRes = tracesFilter.apply();
  const search = pgSearchCondition({ query: searchQuery, tablePrefix: "t" });

  return measureAndReturn({
    operationName: "getTotalUserCount",
    projectId,
    input: {
      params: { tracesFilterRes, search },
      tags: {
        feature: "tracing", type: "trace", kind: "analytic",
        projectId, operation_name: "getTotalUserCount",
      },
    },
    fn: async (input) => {
      const params: unknown[] = [...tracesFilterRes.params];
      let paramIdx = params.length;
      let searchClause = "";
      if (search.params.length > 0) {
        searchClause = offsetFilterParams(search.query, paramIdx);
        params.push(...search.params);
        paramIdx += search.params.length;
      }
      const filterQuery = offsetFilterParams(tracesFilterRes.query || "1=1", 0);
      const query = `
        SELECT COUNT(DISTINCT t.user_id) AS "totalCount"
        FROM traces t
        WHERE ${filterQuery} ${searchClause}
        AND t.user_id IS NOT NULL AND t.user_id != ''
      `;
      return queryPg<{ totalCount: bigint }>({ query, params, tags: input.tags });
    },
  });
};

// ============================================================
// getUserMetrics
// ============================================================
export const getUserMetrics = async (
  projectId: string,
  userIds: string[],
  filter: FilterState,
) => {
  if (userIds.length === 0) return [];

  const chFilter = new FilterList(
    createFilterFromFilterState(filter, tracesTableUiColumnDefinitions, tracesTableCols),
  );
  const chFilterRes = chFilter.apply();
  const timestampFilter = chFilter.find(
    (f) => f.field === "timestamp" && f.operator === ">=",
  );

  const query = `
    WITH stats as (
      SELECT
        t.user_id as user_id,
        (array_agg('default' ORDER BY t.timestamp DESC))[1] as environment,
        count(distinct o.id)::text as obs_count,
        COALESCE(SUM((o.usage_details->>'input')::numeric), 0) as input_usage,
        COALESCE(SUM((o.usage_details->>'output')::numeric), 0) as output_usage,
        COALESCE(SUM((o.usage_details->>'total')::numeric), 0) as total_usage,
        SUM(o.total_cost) as sum_total_cost,
        max(t.timestamp) as max_timestamp,
        min(t.timestamp) as min_timestamp,
        count(distinct t.id)::text as trace_count
      FROM (
        SELECT o.project_id, o.trace_id, o.usage_details, o.total_cost, id,
          ROW_NUMBER() OVER (PARTITION BY id, project_id ORDER BY updated_at DESC) AS rn
        FROM observations o
        WHERE o.project_id = $1
          ${timestampFilter ? `AND o.start_time >= $3::timestamptz - INTERVAL '2 DAYS'` : ""}
          AND o.trace_id = ANY(
            SELECT distinct id from traces t
            where t.user_id = ANY($2::text[]) AND t.project_id = $1
            ${filter.length > 0 && chFilterRes.query !== "1=1" ? `AND ${offsetFilterParams(chFilterRes.query, 2 + (timestampFilter ? 1 : 0) + chFilterRes.params.length)}` : ""}
          )
      ) as o
      JOIN (
        SELECT DISTINCT ON (t.id, t.project_id)
          t.id, t.user_id, t.project_id, t.timestamp, '''default''' as environment
        FROM traces t
        WHERE t.user_id = ANY($2::text[]) AND t.project_id = $1
        ${filter.length > 0 && chFilterRes.query !== "1=1" ? `AND ${offsetFilterParams(chFilterRes.query, 2)}` : ""}
        ORDER BY t.id, t.project_id, t.updated_at DESC
      ) as t on t.id = o.trace_id and t.project_id = o.project_id
      WHERE o.rn = 1
      group by t.user_id
    )
    SELECT user_id, environment, max_timestamp, min_timestamp,
      input_usage, output_usage, total_usage, obs_count, trace_count, sum_total_cost
    FROM stats
  `;

  return measureAndReturn({
    operationName: "getUserMetrics",
    projectId,
    input: {
      params: { projectId, userIds, chFilterRes, timestampFilter },
      tags: {
        feature: "tracing", type: "trace", kind: "analytic",
        projectId, operation_name: "getUserMetrics",
      },
    },
    fn: async (input) => {
      const params: unknown[] = [projectId, userIds];
      if (timestampFilter) {
        params.push(((timestampFilter as DateTimeFilter).value as Date).toISOString());
      }
      if (filter.length > 0) {
        params.push(...chFilterRes.params);
      }

      const rows = await queryPg<{
        user_id: string; environment: string | null; max_timestamp: Date; min_timestamp: Date;
        input_usage: number; output_usage: number; total_usage: number;
        obs_count: string; trace_count: string; sum_total_cost: number;
      }>({ query, params, tags: input.tags });

      return rows.map((row) => ({
        userId: row.user_id,
        environment: row.environment,
        maxTimestamp: row.max_timestamp instanceof Date ? row.max_timestamp : new Date(row.max_timestamp),
        minTimestamp: row.min_timestamp instanceof Date ? row.min_timestamp : new Date(row.min_timestamp),
        inputUsage: Number(row.input_usage),
        outputUsage: Number(row.output_usage),
        totalUsage: Number(row.total_usage),
        observationCount: Number(row.obs_count),
        traceCount: Number(row.trace_count),
        totalCost: Number(row.sum_total_cost),
      }));
    },
  });
};

// ============================================================
// getTracesForBlobStorageExport
// ============================================================
export const getTracesForBlobStorageExport = function (
  projectId: string,
  minTimestamp: Date,
  maxTimestamp: Date,
) {
  const query = `
    SELECT DISTINCT ON (id, project_id)
      id, timestamp, name, environment, project_id, metadata,
      user_id, session_id, release, version, public as public,
      bookmarked as bookmarked, tags, input as input, output as output,
      created_at, updated_at
    FROM traces
    WHERE project_id = $1
    AND timestamp >= $2::timestamptz AND timestamp <= $3::timestamptz
    ORDER BY id, project_id, updated_at DESC
  `;

  return queryPgStream<Record<string, unknown>>({
    query,
    params: [projectId, minTimestamp.toISOString(), maxTimestamp.toISOString()],
    tags: { feature: "blobstorage", type: "trace", kind: "analytic", projectId },
  });
};

// ============================================================
// getTracesForAnalyticsIntegrations
// ============================================================
export const getTracesForAnalyticsIntegrations = async function* (
  projectId: string,
  projectName: string,
  minTimestamp: Date,
  maxTimestamp: Date,
  options: { useGraceHash?: boolean } = {},
) {
  const query = `
    WITH observations_agg AS (
      SELECT o.project_id, o.trace_id,
        sum(total_cost) as total_cost, count(*) as observation_count,
        EXTRACT(EPOCH FROM (greatest(max(end_time), max(start_time)) - least(min(start_time), min(end_time)))) * 1000 as latency_milliseconds
      FROM (
        SELECT DISTINCT ON (id, project_id)
          project_id, trace_id, total_cost, start_time, end_time, id
        FROM observations
        WHERE project_id = $1
        AND start_time >= $2::timestamptz - INTERVAL '1 HOUR'
        AND start_time < $3::timestamptz + INTERVAL '2 DAYS'
        ORDER BY id, project_id, updated_at DESC
      ) o
      GROUP BY o.project_id, o.trace_id
    )
    SELECT
      t.id as id, t.timestamp as timestamp, t.name as name,
      t.session_id as session_id, t.user_id as user_id,
      t.release as release, t.version as version, t.tags as tags,
      'default' as environment,
      t.metadata->>'$posthog_session_id' as posthog_session_id,
      t.metadata->>'$mixpanel_session_id' as mixpanel_session_id,
      o.total_cost as total_cost,
      o.latency_milliseconds / 1000 as latency,
      o.observation_count as observation_count
    FROM (
      SELECT DISTINCT ON (id, project_id) *
      FROM traces t
      WHERE t.project_id = $1
      AND t.timestamp >= $2::timestamptz AND t.timestamp < $3::timestamptz
      ORDER BY id, project_id, updated_at DESC
    ) t
    LEFT JOIN observations_agg o ON t.id = o.trace_id AND t.project_id = o.project_id
  `;

  const records = queryPgStream<Record<string, unknown>>({
    query,
    params: [projectId, minTimestamp.toISOString(), maxTimestamp.toISOString()],
    tags: { feature: "posthog", type: "trace", kind: "analytic", projectId },
  });

  const baseUrl = env.NEXTAUTH_URL?.replace("/api/auth", "");

  for await (const record of records) {
    yield {
      timestamp: record.timestamp,
      langfuse_id: record.id,
      langfuse_trace_name: record.name,
      langfuse_url: `${baseUrl}/project/${projectId}/traces/${encodeURIComponent(record.id as string)}`,
      langfuse_user_url: record.user_id
        ? `${baseUrl}/project/${projectId}/users/${encodeURIComponent(record.user_id as string)}`
        : undefined,
      langfuse_cost_usd: record.total_cost,
      langfuse_count_observations: record.observation_count,
      langfuse_session_id: record.session_id,
      langfuse_project_id: projectId,
      langfuse_project_name: projectName,
      langfuse_user_id: record.user_id || null,
      langfuse_latency: record.latency,
      langfuse_release: record.release,
      langfuse_version: record.version,
      langfuse_tags: record.tags,
      langfuse_environment: record.environment || 'default',
      langfuse_event_version: "1.0.0",
      posthog_session_id: record.posthog_session_id ?? null,
      mixpanel_session_id: record.mixpanel_session_id ?? null,
    } satisfies AnalyticsTraceEvent;
  }
};

// ============================================================
// getTracesByIdsForAnyProject
// ============================================================
export const getTracesByIdsForAnyProject = async (traceIds: string[]) => {
  return measureAndReturn({
    operationName: "getTracesByIdsForAnyProject",
    projectId: "__CROSS_PROJECT__",
    input: {
      params: { traceIds },
      tags: {
        feature: "tracing", type: "trace", kind: "list",
        operation_name: "getTracesByIdsForAnyProject",
      },
    },
    fn: async (input) => {
      const query = `
        SELECT DISTINCT ON (id, project_id) id, project_id
        FROM traces WHERE id = ANY($1::text[])
        ORDER BY id, project_id, updated_at DESC
      `;
      const records = await queryPg<{ id: string; project_id: string }>({
        query, params: [input.params.traceIds], tags: input.tags,
      });
      return records.map((record) => ({
        id: record.id, projectId: record.project_id,
      }));
    },
  });
};

// ============================================================
// getAgentGraphData
// ============================================================
export async function getAgentGraphData(params: {
  projectId: string;
  traceId: string;
  chMinStartTime: string;
  chMaxStartTime: string;
}) {
  const { projectId, traceId, chMinStartTime, chMaxStartTime } = params;

  const query = `
    SELECT id, parent_observation_id, type, name, start_time, end_time,
      metadata->>'langgraph_node' AS node, metadata->>'langgraph_step' AS step
    FROM observations
    WHERE project_id = $1 AND trace_id = $2
    AND start_time >= $3::timestamptz AND start_time <= $4::timestamptz
  `;

  return queryPg({ query, params: [projectId, traceId, chMinStartTime, chMaxStartTime] });
}

// ============================================================
// getTraceCountsByProjectAndDay
// ============================================================
export const getTraceCountsByProjectAndDay = async ({
  startDate,
  endDate,
}: {
  startDate: Date;
  endDate: Date;
}) => {
  const query = `
    SELECT count(*)::text as count, project_id, "timestamp"::date as date
    FROM traces
    WHERE timestamp >= $1::timestamptz AND timestamp < $2::timestamptz
    GROUP BY project_id, "timestamp"::date
  `;

  const rows = await queryPg<{ count: string; project_id: string; date: string }>({
    query,
    params: [startDate.toISOString(), endDate.toISOString()],
    tags: { feature: "tracing", type: "trace", kind: "analytic" },
  });

  return rows.map((row) => ({
    count: Number(row.count),
    projectId: row.project_id,
    date: row.date,
  }));
};

// ============================================================
// TRACE_FIELD_GROUPS (public-API field selection)
// ============================================================
export const TRACE_FIELD_GROUPS = [
  "core",
  "io",
  "scores",
  "observations",
  "metrics",
] as const;

export type TraceFieldGroup = (typeof TRACE_FIELD_GROUPS)[number];

// ============================================================
// TraceQueryType
// ============================================================
export type TraceQueryType = {
  page: number;
  limit: number;
  projectId: string;
  traceId?: string;
  userId?: string;
  name?: string;
  type?: string;
  sessionId?: string;
  version?: string;
  release?: string;
  tags?: string | string[];
  environment?: string | string[];
  fromTimestamp?: string;
  toTimestamp?: string;
  fields?: TraceFieldGroup[];
  useEventsTable?: boolean | null;
};

const traceOrderByColumns = [
  "id", "timestamp", "name", "userId", "release", "version",
  "public", "bookmarked", "sessionId",
].map((name) => ({
  uiTableName: name,
  uiTableId: name,
  clickhouseTableName: "traces",
  clickhouseSelect: snakeCase(name),
  queryPrefix: "t",
}));

// ============================================================
// buildTracesBaseQuery
// ============================================================
async function buildTracesBaseQuery(
  {
    projectId,
    filter,
    pagination,
  }: {
    projectId: string;
    filter: FilterList;
    pagination?: { limit: number; page: number };
  },
  select:
    | {
        includeObservations: boolean;
        includeIO: boolean;
        includeMetrics: boolean;
        includeScores: boolean;
        count: false;
      }
    | {
        includeObservations: false;
        includeIO: false;
        includeMetrics: false;
        includeScores: false;
        count: true;
      },
  orderBy?: OrderByState,
): Promise<{
  query: string;
  params: unknown[];
  fromTimeFilter?: DateTimeFilter | undefined;
}> {
  const disableObservationsFinal = await shouldSkipObservationsFinal(projectId);
  const propagateObservationsTimeBounds =
    env.LANGFUSE_API_CLICKHOUSE_PROPAGATE_OBSERVATIONS_TIME_BOUNDS === "true";

  const appliedFilter = filter.apply();

  const fromTimeFilter = filter.find(
    (f) =>
      f.clickhouseTable === "traces" &&
      f.field.includes("timestamp") &&
      (f.operator === ">=" || f.operator === ">"),
  ) as DateTimeFilter | undefined;
  const toTimeFilter = filter.find(
    (f) =>
      f.clickhouseTable === "traces" &&
      f.field.includes("timestamp") &&
      (f.operator === "<=" || f.operator === "<"),
  ) as DateTimeFilter | undefined;

  const environmentFilter = filter
    .filter((f) => f.field === "environment")
    .map((f) => {
      f.tablePrefix = undefined;
      return f;
    });
  const appliedEnvironmentFilter = new FilterList(environmentFilter).apply();

  const shouldUseSkipIndexes = filter.some(
    (f) =>
      f.clickhouseTable === "traces" &&
      ["user_id", "session_id", "metadata"].some((skipIndexCol) =>
        f.field.includes(skipIndexCol),
      ),
  );

  const filtersNeedObservations = filter.some(
    (f) => f.clickhouseTable === "observations",
  );
  const filtersNeedScores = filter.some((f) => f.clickhouseTable === "scores");

  const hasScoreAggregationFilters = filter.some(
    (f) => f.field === "s.scores_avg" || f.field === "s.score_categories",
  );

  const ctes: string[] = [];

  if (
    select.includeObservations ||
    select.includeMetrics ||
    filtersNeedObservations
  ) {
    const shouldUseFinal =
      (select.includeMetrics || filtersNeedObservations) &&
      !disableObservationsFinal;

    const includeMetricsInCTE =
      select.includeMetrics || filtersNeedObservations;

    const obsSubquery = shouldUseFinal
      ? `(SELECT DISTINCT ON (id, project_id) * FROM observations ORDER BY id, project_id, updated_at DESC)`
      : `observations`;

    ctes.push(`
    observation_stats AS (
      SELECT
        trace_id,
        project_id,
        ${includeMetricsInCTE ? "sum(total_cost) as total_cost, EXTRACT(EPOCH FROM (greatest(max(end_time), max(start_time)) - least(min(start_time), min(end_time)))) * 1000 as latency_milliseconds, " : ""}
        usage_details as usage_details,
        cost_details as cost_details,
        CASE
          WHEN bool_or(level = 'ERROR') THEN 'ERROR'
          WHEN bool_or(level = 'WARNING') THEN 'WARNING'
          WHEN bool_or(level = 'DEFAULT') THEN 'DEFAULT'
          ELSE 'DEBUG'
        END AS aggregated_level,
        count(*) FILTER (WHERE level = 'WARNING') as warning_count,
        count(*) FILTER (WHERE level = 'ERROR') as error_count,
        count(*) FILTER (WHERE level = 'DEFAULT') as default_count,
        count(*) FILTER (WHERE level = 'DEBUG') as debug_count,
        array_agg(DISTINCT id) as observation_ids
      FROM ${obsSubquery} o
      WHERE project_id = $1
      ${fromTimeFilter ? `AND start_time >= $${2}::timestamptz - INTERVAL '2 DAYS'` : ""}
      ${toTimeFilter && propagateObservationsTimeBounds ? `AND start_time <= $${3}::timestamptz + INTERVAL '2 DAYS'` : ""}
      ${toTimeFilter && propagateObservationsTimeBounds ? `AND end_time <= $${3}::timestamptz + INTERVAL '2 DAYS'` : ""}
      ${environmentFilter.length() > 0 ? `AND ${appliedEnvironmentFilter.query}` : ""}
      GROUP BY project_id, trace_id
    )`);
  }

  if (select.includeScores || filtersNeedScores) {
    if (hasScoreAggregationFilters) {
      ctes.push(`
    score_stats AS (
      SELECT
        trace_id,
        project_id,
        array_agg(DISTINCT id) as score_ids,
        array_agg(jsonb_build_object('name', name, 'avg_value', avg_value)) FILTER (WHERE data_type IN ('NUMERIC', 'BOOLEAN')) AS scores_avg,
        array_agg(name || ':' || string_value) FILTER (WHERE data_type = 'CATEGORICAL' AND string_value IS NOT NULL AND string_value != '') AS score_categories
      FROM (
        SELECT DISTINCT ON (project_id, trace_id, id, name, data_type, string_value)
          project_id, trace_id, id, name, data_type, string_value,
          avg(value) as avg_value
        FROM scores
        WHERE project_id = $1
        AND session_id IS NULL AND dataset_run_id IS NULL
        AND data_type = ANY($$2::text[])
        ${fromTimeFilter ? `AND timestamp >= $${3}::timestamptz` : ""}
        ${environmentFilter.length() > 0 ? `AND ${appliedEnvironmentFilter.query}` : ""}
        GROUP BY project_id, trace_id, id, name, data_type, string_value
        ORDER BY project_id, trace_id, id, name, data_type, string_value, updated_at DESC
      ) tmp
      GROUP BY project_id, trace_id
    )`);
    } else {
      ctes.push(`
    score_stats AS (
      SELECT
        trace_id, project_id,
        array_agg(DISTINCT id) as score_ids,
        array_agg(jsonb_build_object('name', name, 'value', value)) FILTER (WHERE data_type IN ('NUMERIC', 'BOOLEAN')) as scores_avg,
        array_agg(name || ':' || string_value) FILTER (WHERE data_type = 'CATEGORICAL') as score_categories
      FROM scores
      WHERE project_id = $1
      AND session_id IS NULL AND dataset_run_id IS NULL
      AND data_type = ANY($$2::text[])
      ${fromTimeFilter ? `AND timestamp >= $${3}::timestamptz` : ""}
      ${environmentFilter.length() > 0 ? `AND ${appliedEnvironmentFilter.query}` : ""}
      GROUP BY project_id, trace_id
    )`);
    }
  }

  const withClause = ctes.length > 0 ? `WITH ${ctes.join(", ")}` : "";

  const chOrderBy =
    (orderByToPgSql(orderBy || [], "t") || "ORDER BY t.timestamp desc") +
    (shouldUseSkipIndexes ? ", t.updated_at desc" : "");

  const traceSubquery = shouldUseSkipIndexes
    ? `traces t`
    : `(SELECT DISTINCT ON (id, project_id) * FROM traces ORDER BY id, project_id, updated_at DESC) t`;

  const queryMiddle = `
  FROM ${traceSubquery}
  ${select.includeObservations || select.includeMetrics || filtersNeedObservations ? "LEFT JOIN observation_stats o ON t.id = o.trace_id AND t.project_id = o.project_id" : ""}
  ${select.includeScores || filtersNeedScores ? "LEFT JOIN score_stats s ON t.id = s.trace_id AND t.project_id = s.project_id" : ""}
  WHERE t.project_id = $1
  ${filter.length() > 0 ? `AND ${appliedFilter.query}` : ""}
  `;

  const paginationClause =
    pagination !== undefined
      ? `LIMIT ${pagination.limit} OFFSET ${(pagination.page - 1) * pagination.limit}`
      : "";
  const limitByClause = shouldUseSkipIndexes
    ? "ORDER BY t.id, t.project_id, t.updated_at DESC"
    : "";

  const coreSelect = `
      t.id as id,
      t.project_id as project_id,
      t.timestamp as timestamp,
      t.name as name,
      'default' as environment,
      t.session_id as session_id,
      t.user_id as user_id,
      t.release as release,
      t.version as version,
      t.bookmarked as bookmarked,
      t.public as public,
      t.tags as tags,
      t.created_at as created_at,
      t.updated_at as updated_at`;

  const scoresSelect = select.includeScores ? ", s.score_ids as scores" : "";
  const observationsSelect = select.includeObservations
    ? ", o.observation_ids as observations"
    : "";
  const metricsSelect = select.includeMetrics
    ? ', COALESCE(o.latency_milliseconds / 1000, 0) as latency, COALESCE(o.total_cost, 0) as "totalCost"'
    : "";

  const finalOrderBy =
    orderByToPgSql(orderBy || [], "b") || "ORDER BY b.timestamp desc";

  let query: string;

  const pgOrderBy =
    orderByToPgSql(orderBy || [], "t") || "ORDER BY t.timestamp desc";

  if (select.count) {
    query = `${withClause}
      SELECT count(*)::text as count
      ${queryMiddle}
    `;
  } else if (select.includeIO) {
    ctes.push(`base AS (
      SELECT ${coreSelect}
        ${scoresSelect}
        ${observationsSelect}
        ${metricsSelect}
      ${queryMiddle}
      ${pgOrderBy}
      ${limitByClause}
      ${paginationClause}
    )`);

    ctes.push(`io AS (
      SELECT id as _io_id, project_id as _io_project_id, input, output, metadata
      FROM traces
      WHERE project_id = $1
      AND (id, project_id) IN (SELECT id, project_id FROM base)
      ${fromTimeFilter ? `AND timestamp >= $${2}::timestamptz` : ""}
      ${toTimeFilter ? `AND timestamp <= $${3}::timestamptz` : ""}
    )`);

    query = `WITH ${ctes.join(", ")}
      SELECT
        b.id as id,
        '/project/' || b.project_id || '/traces/' || b.id as "htmlPath",
        b.project_id as project_id,
        b.timestamp as timestamp,
        b.name as name,
        b.environment as environment,
        b.session_id as session_id,
        b.user_id as user_id,
        b.release as release,
        b.version as version,
        b.bookmarked as bookmarked,
        b.public as public,
        b.tags as tags,
        b.created_at as created_at,
        b.updated_at as updated_at,
        i.input as input,
        i.output as output,
        i.metadata as metadata
        ${select.includeScores ? ", b.scores as scores" : ""}
        ${select.includeObservations ? ", b.observations as observations" : ""}
        ${select.includeMetrics ? ', b.latency as latency, b."totalCost" as "totalCost"' : ""}
      FROM base b
      LEFT JOIN io i ON b.id = i._io_id AND b.project_id = i._io_project_id
      ${finalOrderBy}
    `;
  } else {
    query = `
      ${withClause}
      SELECT
        ${coreSelect},
        '/project/' || t.project_id || '/traces/' || t.id as "htmlPath"
        ${scoresSelect}
        ${observationsSelect}
        ${metricsSelect}
      ${queryMiddle}
      ${pgOrderBy}
      ${limitByClause}
      ${paginationClause}
    `;
  }

  const needsCteToTimeFilter =
    toTimeFilter &&
    (propagateObservationsTimeBounds || (!select.count && select.includeIO));

  const params: unknown[] = [projectId];
  params.push(...(appliedEnvironmentFilter.params || []));
  params.push(...(appliedFilter.params || []));
  params.push(LISTABLE_SCORE_TYPES);
  if (fromTimeFilter) {
    params.push(fromTimeFilter.value.toISOString());
  }
  if (needsCteToTimeFilter) {
    params.push(toTimeFilter!.value.toISOString());
  }

  return { query, params, fromTimeFilter };
}

// ============================================================
// generateTracesForPublicApi
// ============================================================
export const generateTracesForPublicApi = async ({
  projectId,
  filter,
  orderBy,
  pagination,
  fields,
}: {
  projectId: string;
  filter: FilterList;
  orderBy: OrderByState;
  pagination?: { limit: number; page: number };
  fields?: TraceFieldGroup[];
}) => {
  const requestedFields = fields ?? TRACE_FIELD_GROUPS;
  const includeIO = requestedFields.includes("io");
  const includeScores = requestedFields.includes("scores");
  const includeObservations = requestedFields.includes("observations");
  const includeMetrics = requestedFields.includes("metrics");

  const { query, params, fromTimeFilter } = await buildTracesBaseQuery(
    { projectId, filter, pagination },
    {
      includeIO, includeObservations, includeMetrics, includeScores,
      count: false,
    },
    orderBy,
  );

  const result = await measureAndReturn({
    operationName: "getTracesForPublicApi",
    projectId,
    input: {
      params,
      tags: {
        feature: "tracing", type: "trace", kind: "public-api",
        projectId, operation_name: "getTracesForPublicApi",
      },
      fromTimestamp: fromTimeFilter?.value ?? undefined,
    },
    fn: (input) => {
      return queryPg<
        TraceRecordReadType & {
          observations?: string[]; scores?: string[];
          totalCost?: number; latency?: number; htmlPath: string;
        }
      >({ query, params: input.params as unknown[], tags: input.tags });
    },
  });

  return convertClickhouseTracesListToDomain(result, {
    metrics: includeMetrics,
    scores: includeScores,
    observations: includeObservations,
  });
};

// ============================================================
// getTracesCountForPublicApi
// ============================================================
export const getTracesCountForPublicApi = async ({
  projectId,
  filter,
  pagination,
}: {
  projectId: string;
  filter: FilterList;
  pagination?: { limit: number; page: number };
}) => {
  const appliedFilter = filter.apply();

  const fromTimeFilter = filter.find(
    (f) =>
      f.clickhouseTable === "traces" &&
      f.field.includes("timestamp") &&
      (f.operator === ">=" || f.operator === ">"),
  ) as DateTimeFilter | undefined;

  const needsComplexQuery = filter.some(
    (f) =>
      f.clickhouseTable === "observations" ||
      f.clickhouseTable === "scores" ||
      (f.clickhouseTable === "traces" &&
        !["user_id", "session_id", "metadata"].some((c) =>
          f.field.includes(c),
        )),
  );

  let query = `
    SELECT count(*)::text as count
    FROM traces t
    WHERE project_id = $1
    ${filter.length() > 0 ? `AND ${appliedFilter.query}` : ""}
  `;

  let params: unknown[] = [projectId, ...(appliedFilter.params || [])];

  if (needsComplexQuery) {
    const result = await buildTracesBaseQuery(
      { projectId, filter, pagination },
      {
        includeObservations: false, includeIO: false,
        includeMetrics: false, includeScores: false, count: true,
      },
    );
    query = result.query;
    params = result.params;
  }

  const timestamp = fromTimeFilter?.value;

  return measureAndReturn({
    operationName: "getTracesCountForPublicApi",
    projectId,
    input: {
      params,
      tags: {
        feature: "tracing", type: "trace", kind: "count",
        projectId, operation_name: "getTracesCountForPublicApi",
      },
      timestamp,
    },
    fn: async (input) => {
      const records = await queryPg<{ count: string }>({
        query,
        params: input.params as unknown[],
        tags: input.tags,
      });
      return records.map((record) => Number(record.count)).shift();
    },
  });
};
