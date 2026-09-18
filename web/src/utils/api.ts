/**
 * Rust REST API React Query 客户端
 *
 * 替代 tRPC，所有数据通过 Rust 后端（axum + sqlx）提供。
 * 保持与 tRPC api 对象相同的调用形状，兼容现有 300+ 文件。
 *
 * Rust API 响应格式: { data: T | null, meta: { cursor, has_more, total } | null, error: string | null }
 * 本客户端自动解包 data 字段，meta 合并到返回值的 _meta 字段。
 */

import {
  useQuery,
  useMutation,
  useQueryClient,
  type UseQueryOptions,
  type UseMutationOptions,
} from "@tanstack/react-query";
import Decimal from "decimal.js";

// ============================================================================
// Rust API 响应类型 & fetch 工具
// ============================================================================

interface RustResponse<T = any> {
  data: T | null;
  meta: { cursor: string | null; has_more: boolean; total: number | null } | null;
  error: string | null;
}

/** 向 Rust 后端发 GET 请求（通过 Next.js 代理，same-origin） */

// Auto-convert ISO date strings in API responses to Date objects
// Also converts snake_case keys to camelCase (Rust API → JS convention)
function snakeToCamel(str: string): string {
  return str.replace(/_([a-z])/g, (_, c) => c.toUpperCase());
}

// Reverse of snakeToCamel: converts camelCase keys back to snake_case
// for API request query params (Rust backend expects snake_case)
function camelToSnake(str: string): string {
  return str.replace(/[A-Z]/g, (c) => '_' + c.toLowerCase());
}

/**
 * Wrap a plain number back into a `Decimal`.
 *
 * Components written against tRPC call `.toNumber()` on cost fields, because
 * superjson preserved the `Decimal` class across the wire; a JSON response only
 * carries a number, so those calls would throw. `undefined` stays `undefined`
 * so an unpriced row keeps meaning "no cost" rather than "zero".
 */
function reviveDecimal(value: unknown): Decimal | undefined {
  if (value === null || value === undefined) return undefined;
  return new Decimal(value as Decimal.Value);
}

/**
 * Objects whose keys are usage types rather than field names.
 *
 * `snakeToCamel` is right for field names but wrong for these: their keys are
 * data (`cache_read_input_tokens`, `input_cache_creation`, …), and renaming them
 * makes the UI show spellings that match neither what the client reported nor
 * what `prices` is keyed by. They are carried through verbatim.
 */
const USAGE_TYPE_KEYED = new Set([
  "usage_details",
  "cost_details",
  "provided_usage_details",
  "provided_cost_details",
  "prices",
]);

function autoParseDates(obj: any): any {
  if (obj === null || obj === undefined) return obj;
  if (typeof obj === "string" && /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}/.test(obj)) {
    return new Date(obj);
  }
  if (Array.isArray(obj)) return obj.map(autoParseDates);
  if (typeof obj === "object") {
    const result: Record<string, any> = {};
    for (const [k, v] of Object.entries(obj)) {
      const camelKey = snakeToCamel(k);
      result[camelKey] = USAGE_TYPE_KEYED.has(k) ? v : autoParseDates(v);
    }
    return result;
  }
  return obj;
}

async function rustGet<T = any>(
  path: string,
  params?: Record<string, any>,
): Promise<T> {
  const url = new URL(path, window.location.origin);
  if (params) {
    for (const [k, v] of Object.entries(params)) {
      if (v !== undefined && v !== null && v !== "") {
        const val = (Array.isArray(v) || typeof v === "object") ? JSON.stringify(v) : String(v);
        url.searchParams.set(camelToSnake(k), val);
      }
    }
  }

  const res = await fetch(url.toString(), { credentials: "include" });
  if (!res.ok) {
    const text = await res.text().catch(() => "Server error");
    throw new Error(`API ${res.status}: ${text.slice(0, 200)}`);
  }
  const json: RustResponse<T> = await res.json();
  if (json.error) throw new Error(json.error);
  const data = autoParseDates(json.data) as any;
  if (json.meta) data._meta = json.meta;
  return data as T;
}

/** 向 Rust 后端发 mutation 请求 */
async function rustMutate<T = any>(
  method: string,
  path: string,
  body?: any,
  params?: Record<string, any>,
): Promise<T> {
  const url = new URL(path, window.location.origin);
  if (params) {
    for (const [k, v] of Object.entries(params)) {
      if (v !== undefined && v !== null && v !== "") {
        const val = (Array.isArray(v) || typeof v === "object") ? JSON.stringify(v) : String(v);
        url.searchParams.set(camelToSnake(k), val);
      }
    }
  }

  const res = await fetch(url.toString(), {
    method,
    headers: { "Content-Type": "application/json" },
    credentials: "include",
    body: body !== undefined ? JSON.stringify(body) : undefined,
  });
  if (!res.ok) {
    const text = await res.text().catch(() => "Server error");
    throw new Error(`API ${res.status}: ${text.slice(0, 200)}`);
  }
  const json: RustResponse<T> = await res.json();
  if (json.error) throw new Error(json.error);
  return autoParseDates(json.data) as T;
}

// ============================================================================
// Query Key Builder
// ============================================================================

/** 生成与 tRPC 兼容的 queryKey */
function qKey(resource: string, procedure: string, input?: any): any[] {
  const key: any[] = [resource, procedure];
  if (input !== undefined && input !== null) {
    key.push(input);
  }
  return key;
}

// ============================================================================
// Query Procedure Factory (GET with optional params)
// ============================================================================

/**
 * 创建一个 GET 查询过程
 * @param path 不带参数的 API 路径，如 "/api/traces"
 * @param resource 用于 queryKey 的资源名
 * @param procedure 过程名
 * @param extractParams 从 input 提取查询参数
 */
function queryProc<TOutput = any>(
  path: string,
  resource: string,
  procedure: string,
  extractParams: (input: any) => Record<string, any> = (i) => i ?? {},
) {
  return {
    useQuery(
      input?: any,
      opts?: UseQueryOptions<TOutput, Error>,
    ) {
      return useQuery<TOutput, Error>({
        queryKey: qKey(resource, procedure, input),
        queryFn: () => rustGet<TOutput>(path, extractParams(input)),
        ...opts,
      } as any);
    },
    useInfiniteQuery(
      input?: any,
      opts?: any,
    ) {
      // Simple implementation: use regular query, no infinite scroll support yet
      return useQuery<TOutput, Error>({
        queryKey: qKey(resource, procedure, input),
        queryFn: () => rustGet<TOutput>(path, extractParams(input)),
        ...opts,
      } as any);
    },
    useSuspenseQuery(
      input?: any,
      opts?: UseQueryOptions<TOutput, Error>,
    ) {
      return useQuery<TOutput, Error>({
        queryKey: qKey(resource, procedure, input),
        queryFn: () => rustGet<TOutput>(path, extractParams(input)),
        ...opts,
      } as any);
    },
  };
}

// ============================================================================
// Single-item Query Procedure (GET /api/{resource}/{id})
// ============================================================================

function queryByIdProc<TOutput = any>(
  pathPattern: string,
  resource: string,
  procedure: string,
  idKey: string = "id",
) {
  const hook = {
    useQuery(
      input: any,
      opts?: UseQueryOptions<TOutput, Error>,
    ) {
      const id = input?.[idKey];
      const params = { ...input };
      delete params[idKey];
      return useQuery<TOutput, Error>({
        queryKey: qKey(resource, procedure, { id, ...params }),
        queryFn: () => {
          if (!id) return Promise.resolve(null as any);
          return rustGet<TOutput>(`${pathPattern}/${encodeURIComponent(id)}`, params);
        },
        enabled: !!id && (opts?.enabled !== false),
        ...opts,
      } as any);
    },
    useInfiniteQuery(input: any, opts?: any) {
      return hook.useQuery(input, opts);
    },
    useSuspenseQuery(input: any, opts?: UseQueryOptions<TOutput, Error>) {
      return hook.useQuery(input, opts);
    },
  };
  return hook;
}

// ============================================================================
// Mutation Procedure Factory
// ============================================================================

function mutationProc<TInput = any, TOutput = any>(
  method: string,
  path: string,
  resource: string,
  procedure: string,
  buildPath?: (input: any) => string,
  buildParams?: (input: any) => Record<string, any>,
  buildBody?: (input: any) => any,
) {
  return {
    useMutation(
      opts?: UseMutationOptions<TOutput, Error, TInput>,
    ) {
      const qc = useQueryClient();
      return useMutation<TOutput, Error, TInput>({
        mutationFn: async (input: any) => {
          const url = buildPath ? buildPath(input) : path;
          const params = buildParams ? buildParams(input) : {};
          const body = buildBody ? buildBody(input) : input;
          return rustMutate<TOutput>(method, url, body, params);
        },
        onSuccess: (data: any, variables: any, context: any) => {
          qc.invalidateQueries({ queryKey: [resource] });
          opts?.onSuccess?.(data, variables, context);
        },
        ...opts,
      } as any);
    },
  };
}

// ============================================================================
// Collection CRUD Procedure Factories (for standard REST resources)
// ============================================================================

function listProc(resource: string, dataKey?: string) {
  return queryProc(`/api/${resource}`, resource, "all", (input: any) => {
    const params: Record<string, any> = {};
    if (!input) return params;
    // Normalize projectId -> project_id
    if (input.projectId) params.project_id = input.projectId;
    if (input.project_id) params.project_id = input.project_id;
    if (input.limit) params.limit = input.limit;
    if (input.cursor) params.cursor = input.cursor;
    if (input.page) params.page = input.page;
    if (input.trace_id) params.trace_id = input.trace_id;
    if (input.traceId) params.trace_id = input.traceId;
    if (input.type) params.type = input.type;
    if (input.name) params.name = input.name;
    // Pass through other filter params
    for (const [k, v] of Object.entries(input)) {
      const _skip = ["projectId", "project_id", "limit", "cursor", "page", "trace_id", "traceId", "type", "name", "filter", "orderBy", "searchType", "fields", "fromTimestamp", "toTimestamp", "userId", "environment", "tags"];
      if (!_skip.includes(k)) {
        if (v !== undefined && v !== null && v !== "") {
          params[k] = (Array.isArray(v) || typeof v === "object") ? JSON.stringify(v) : v;
        }
      }
    }
    return params;
  });
}

function getByIdProc(resource: string) {
  return queryByIdProc(`/api/${resource}`, resource, "byId", "id");
}

function createProc(resource: string) {
  return mutationProc("POST", `/api/${resource}`, resource, "create");
}

function updateProc(resource: string) {
  return mutationProc(
    "PUT",
    `/api/${resource}`,
    resource,
    "update",
    (input: any) => `/api/${resource}/${encodeURIComponent(input?.id ?? "")}`,
    (input: any) => {
      const { id, ...rest } = input ?? {};
      return rest;
    },
    (input: any) => {
      const { id, ...rest } = input ?? {};
      return rest;
    },
  );
}

function deleteProc(resource: string) {
  return mutationProc(
    "DELETE",
    `/api/${resource}`,
    resource,
    "delete",
    (input: any) => `/api/${resource}/${encodeURIComponent(input?.id ?? input?.traceId ?? input?.scoreId ?? "")}`,
    (input: any) => {
      const { id, traceId, scoreId, ...rest } = input ?? {};
      return rest;
    },
  );
}

// ============================================================================
// Placeholder procedure (for operations not yet migrated to Rust)
// ============================================================================

function placeholderProc(resource: string, procedure: string, note?: string) {
  const warnMsg = `[api] ${resource}.${procedure} not yet migrated to Rust backend` +
    (note ? ` (${note})` : "");
  return {
    useQuery(input?: any, _opts?: any) {
      console.warn(warnMsg);
      return {
        data: {} as any,
        isLoading: false,
        isError: false,
        error: null,
        isPending: false,
        isSuccess: true,
        status: "success",
        fetchStatus: "idle",
        refetch: () => Promise.resolve({ data: {} }),
        remove: () => {},
      } as any;
    },
    useInfiniteQuery(_input?: any, _opts?: any) {
      console.warn(warnMsg + " (useInfiniteQuery)");
      return {
        data: undefined,
        isLoading: false,
        isError: false,
        error: null,
        fetchNextPage: () => Promise.resolve(undefined),
        hasNextPage: false,
        isFetchingNextPage: false,
      } as any;
    },
    useSuspenseQuery(_input?: any, _opts?: any) {
      console.warn(warnMsg + " (useSuspenseQuery)");
      return { data: undefined } as any;
    },
    useMutation(_opts?: any) {
      const qc = useQueryClient();
      return useMutation({
        mutationFn: async (_input: any) => {
          console.warn(warnMsg);
          return undefined;
        },
        onSuccess: () => {
          qc.invalidateQueries({ queryKey: [resource] });
        },
      }) as any;
    },
  };
}

// ============================================================================
// The API Object — mirrors the tRPC router tree
// ============================================================================

/** Mock React component wrapper — 替代 tRPC withTRPC */
function withTRPC(Component: React.ComponentType<any>): React.ComponentType<any> {
  return Component;
}

// ============================================================================
// Real procedure generators for resources with Rust REST endpoints
// ============================================================================

// Resources that have real Rust REST API endpoints
const REAL_RESOURCES: Record<string, {
  listProc?: string;   // returns { [key]: [...] }
  getById?: boolean;   // GET /api/{resource}/{id}
  create?: boolean;    // POST /api/{resource}
  update?: boolean;    // PUT /api/{resource}/{id}
  delete?: boolean;    // DELETE /api/{resource}/{id}
}> = {
  traces:        { listProc: "traces", getById: true },
  observations:  { listProc: "observations", getById: true },
  scores:        { listProc: "scores" },
  sessions:      { listProc: "sessions" },
  datasets:      { listProc: "datasets" },
  prompts:       { listProc: "prompts" },
  models:        { listProc: "models" },
  evals:         { listProc: "evals" },
  projects:      { listProc: "projects" },
  organizations: { listProc: "organizations" },
  dashboards:    { listProc: "dashboards" },
  comments:      { listProc: "comments" },
  media:         { listProc: "media" },
  automations:   { listProc: "automations" },
  monitors:      { listProc: "monitors" },
  users:         { listProc: "users" },
};

// Procedure name aliases — common tRPC procedure names that map to REST operations
const LIST_ALIASES = new Set(["all", "list", "getAll", "allDatasets", "allTemplates", "allNames", "allDatasetsMetrics",
  "allDatasetMeta", "allConfigs", "allPromptMeta", "allTemplatesForName", "allNamesAndIds", "globalJobConfigs"]);
const GET_ALIASES = new Set(["byId", "get", "getById", "getAutomation", "getStatus", "getDashboard",
  "getSubscriptionInfo", "getSpendAlerts", "getIntegrationStatus", "getChannels", "getDefault",
  "getDefaultAssignments", "getByTableName", "getForProject", "getByObjectId", "getByTraceOrObservationId",
  "getCountByObjectId", "getCountByObjectType", "getTraceCommentCountsBySessionId",
  "getAgentGraphData", "getFilterOptions", "getIsBatchActionInProgress", "getSdkVersionInfo",
  "getScoreColumns", "getScoreIdentifiers", "getScoreComparisonAnalytics", "getScoreMetadataById",
  "getProtectedLabels", "getPromptLinkOptions", "getCountOfConsecutiveFailures", "getAutomationExecutions",
  "getLogs", "templateById", "configById", "itemById", "runById", "fetchDefaultModel", "templateNames"]);
const CREATE_ALIASES = new Set(["create", "createAutomation", "createDashboard", "createTemplate",
  "createJob", "createDataset", "createExperiment", "createAnnotationScore", "createSpendAlert",
  "createMany", "createManyDatasetItems", "createManyItems", "createSupportThread", "createStripeCheckoutSession"]);
const UPDATE_ALIASES = new Set(["update", "updateAutomation", "updateDashboard", "updateDataset",
  "updateEvalJob", "updateAnnotationScore", "updateTags", "updateSpendAlert", "upsert", "upsertCorrection",
  "upsertDefaultModel", "updateDashboardMetadata", "updateDashboardFilters", "updateDashboardDefinition",
  "updateName", "updateNote", "updateProjectRole", "updateOrgMembership", "setLabels", "setRetention",
  "setAsDefault", "addProtectedLabel", "removeProtectedLabel"]);
const DELETE_ALIASES = new Set(["delete", "deleteAutomation", "deleteDashboard", "deleteDataset",
  "deleteEvalJob", "deleteEvalTemplate", "deleteAnnotationScore", "deleteSpendAlert", "deleteMany",
  "deleteManyItems", "deleteDatasetRuns", "deleteDatasetItem", "deleteVersion", "deleteMembership",
  "deleteInvite", "deleteDefaultModel"]);

function isRealResource(name: string): boolean {
  return name in REAL_RESOURCES;
}

function getRealProc(resource: string, procedure: string): any {
  const res = REAL_RESOURCES[resource];
  if (!res) return null;

  if (LIST_ALIASES.has(procedure) && res.listProc) {
    return listProc(resource, res.listProc);
  }
  if (GET_ALIASES.has(procedure) && res.getById) {
    return getByIdProc(resource);
  }
  if (CREATE_ALIASES.has(procedure) && res.create) {
    return createProc(resource);
  }
  if (UPDATE_ALIASES.has(procedure) && res.update) {
    return updateProc(resource);
  }
  if (DELETE_ALIASES.has(procedure) && res.delete) {
    return deleteProc(resource);
  }
  return null;
}

// ============================================================================
// Auto-placeholder Proxy — handles any resource.procedure not yet migrated
// ============================================================================

function createProcedureProxy(resource: string, procedure: string): any {
  const warnMsg = `[api] ${resource}.${procedure} not yet migrated to Rust backend`;

  // Check if this is a known real resource with a real proc
  
  // Special: projects.environmentFilterOptions — return empty array (unblock traces table)
  if (resource === "projects" && procedure === "environmentFilterOptions") {
    return {
      useQuery(_input?: any, _opts?: any) {
        return { data: [], isLoading: false, isError: false, error: null, isPending: false, isSuccess: true, status: "success", fetchStatus: "idle", refetch: () => Promise.resolve({ data: [] }), remove: () => {} };
      },
    };
  }
  // Special: traces.hasTracingConfigured — always return true (local dev, no API call needed)
  if (resource === "traces" && procedure === "hasTracingConfigured") {
    return {
      useQuery(_input?: any, _opts?: any) {
        return { data: true, isLoading: false, isError: false, error: null, isPending: false, isSuccess: true, status: "success", fetchStatus: "idle", refetch: () => Promise.resolve({ data: true }), remove: () => {} };
      },
    };
  }
  // Special: traces.filterOptions — return empty array (local dev stub)
  if (resource === "traces" && procedure === "filterOptions") {
    return {
      useQuery(_input?: any, _opts?: any) {
        return { data: [] as any, isLoading: false, isError: false, error: null, isPending: false, isSuccess: true, status: "success", fetchStatus: "idle", refetch: () => Promise.resolve({ data: [] }), remove: () => {} };
      },
    };
  }
  // Special: traces.metrics — per-trace roll-ups for the traces table's
  // latency / tokens / cost columns. The table joins the result onto the trace
  // rows by id, and reads `totalCost` as a Decimal.
  if (resource === "traces" && procedure === "metrics") {
    return {
      useQuery(input?: any, opts?: any) {
        const projectId = input?.projectId ?? input?.project_id;
        const traceIds = input?.traceIds ?? input?.trace_ids ?? [];
        const enabled = opts?.enabled ?? true;
        return useQuery({
          ...opts,
          queryKey: qKey(resource, procedure, input),
          queryFn: async () => {
            const rows = await rustGet<any[]>("/api/traces/metrics", {
              project_id: projectId,
              trace_ids: traceIds,
            });
            return rows.map((row) => ({
              ...row,
              // The traces table calls `.toNumber()` on these, which the old
              // tRPC transport could satisfy because superjson preserved the
              // `Decimal` class; JSON only carries numbers, so the proxy — whose
              // job is to reproduce the old call shapes — revives them.
              calculatedInputCost: reviveDecimal(row.calculatedInputCost),
              calculatedOutputCost: reviveDecimal(row.calculatedOutputCost),
              calculatedTotalCost: reviveDecimal(row.calculatedTotalCost),
            }));
          },
          enabled: enabled && !!projectId && traceIds.length > 0,
        }) as any;
      },
      useSuspenseQuery(input?: any, opts?: any) {
        return this.useQuery(input, opts);
      },
    };
  }
  // Special: traces.countAll — fetch list with limit=1 to get meta.total
  if (resource === "traces" && procedure === "countAll") {
    return {
      useQuery(input?: any, _opts?: any) {
        const projectId = input?.projectId ?? input?.project_id;
        return useQuery({
          queryKey: qKey(resource, "countAll", input),
          queryFn: async () => {
            const data = await rustGet<any>(`/api/traces`, { project_id: projectId, limit: 1 });
            return { totalCount: data._meta?.total ?? 0 };
          },
          enabled: !!projectId,
          staleTime: 30_000,
          refetchOnMount: false,
          refetchOnWindowFocus: true,
        }) as any;
      },
    };
  }
  if (resource === "traces" && procedure === "hasTracingConfigured") {
    return {
      useQuery(_input?: any, _opts?: any) {
        return { data: true, isLoading: false, isError: false, error: null, isPending: false, isSuccess: true, status: "success", fetchStatus: "idle", refetch: () => Promise.resolve({ data: true }), remove: () => {} };
      },
    };
  }

  // sessions.hasAny / sessions.hasAnyFromEvents → GET /api/sessions/{hasAny,hasAnyFromEvents}
  // sessions.metrics / sessions.metricsFromEvents → GET /api/sessions/{metrics,metricsFromEvents}
  //
  // The sessions page branches on `hasAny` before rendering its table, so a
  // placeholder here leaves the page stuck on the onboarding empty state even
  // when the project has sessions; the metrics queries fill the table's
  // duration / trace count / cost / token columns.
  if (
    resource === "sessions" &&
    (procedure === "hasAny" ||
      procedure === "hasAnyFromEvents" ||
      procedure === "metrics" ||
      procedure === "metricsFromEvents")
  ) {
    const path = `/api/sessions/${procedure}`;
    const isProbe = procedure === "hasAny" || procedure === "hasAnyFromEvents";
    return {
      useQuery(input?: any, opts?: any) {
        const projectId = input?.projectId ?? input?.project_id;
        // Honour the caller's `enabled`: the page enables exactly one probe and
        // one metrics query depending on the beta flag, and overriding it would
        // fire both.
        const enabled = opts?.enabled ?? true;
        const sessionIds = input?.sessionIds ?? input?.session_ids;
        return useQuery({
          ...opts,
          queryKey: qKey(resource, procedure, input),
          queryFn: async () => {
            if (isProbe) {
              return rustGet<boolean>(path, { project_id: projectId });
            }
            const rows = await rustGet<any[]>(path, {
              project_id: projectId,
              session_ids: sessionIds,
            });
            // The sessions table calls `.toNumber()` on the cost columns, which
            // the old tRPC transport could satisfy because superjson kept
            // `Decimal` instances across the wire. JSON cannot carry a class, so
            // the proxy — whose job is to reproduce the old call shapes —
            // revives them here.
            return rows.map((row) => ({
              ...row,
              inputCost: reviveDecimal(row.inputCost),
              outputCost: reviveDecimal(row.outputCost),
              totalCost: reviveDecimal(row.totalCost),
            }));
          },
          enabled: enabled && !!projectId && (isProbe || !!sessionIds),
        }) as any;
      },
      useSuspenseQuery(input?: any, opts?: any) {
        return this.useQuery(input, opts);
      },
    };
  }

  const realProc = getRealProc(resource, procedure);
  if (realProc) return realProc;

  // Otherwise return placeholder
  return {
    useQuery(input?: any, _opts?: any) {
      console.warn(warnMsg);
      return {
        data: undefined,
        isLoading: false,
        isError: false,
        isPending: false,
        isSuccess: false,
        error: null,
        status: "success",
        fetchStatus: "idle",
        refetch: () => Promise.resolve({ data: undefined }),
        remove: () => {},
      } as any;
    },
    useInfiniteQuery(_input?: any, _opts?: any) {
      console.warn(warnMsg + " (useInfiniteQuery)");
      return {
        data: undefined,
        isLoading: false,
        isError: false,
        error: null,
        fetchNextPage: () => Promise.resolve(undefined),
        hasNextPage: false,
        isFetchingNextPage: false,
      } as any;
    },
    useSuspenseQuery(_input?: any, _opts?: any) {
      console.warn(warnMsg + " (useSuspenseQuery)");
      return { data: undefined } as any;
    },
    useMutation(_opts?: any) {
      const qc = useQueryClient();
      return useMutation({
        mutationFn: async (_input: any) => {
          console.warn(warnMsg);
          return undefined;
        },
        onSuccess: () => {
          qc.invalidateQueries({ queryKey: [resource] });
        },
      }) as any;
    },
  };
}

// Create a nested Proxy that auto-generates procedures
function createResourceProxy(resource: string): any {
  const cache: Record<string, any> = {};

  return new Proxy(
    {},
    {
      get(_target: any, procedure: string) {
        if (typeof procedure !== "string") return undefined;
        // Handle special properties
        if (procedure === "then") return undefined; // prevent Promise-like behavior
        if (procedure === Symbol.toPrimitive) return undefined;

        // Check target for manually overridden procedures first
        if (procedure in _target) return _target[procedure];

        if (!cache[procedure]) {
          cache[procedure] = createProcedureProxy(resource, procedure);
        }
        return cache[procedure];
      },
    },
  );
}

// The root api Proxy
const resourceCache: Record<string, any> = {};

export const api: any = new Proxy(
  {
    withTRPC,
    useUtils() {
      const qc = useQueryClient();
      // Build a tRPC-compatible utils object: utils.<resource>.<procedure>.invalidate()
      function buildProcUtils(resource: string, procedure: string) {
        const queryKey = [resource, procedure];
        return {
          invalidate: () => qc.invalidateQueries({ queryKey }),
          refetch: () => qc.refetchQueries({ queryKey }),
          getData: () => qc.getQueryData(queryKey),
          setData: (data: any) => qc.setQueryData(queryKey, data),
          prefetch: (input: any, opts?: any) => {
            const fullKey = input != null ? [...queryKey, input] : queryKey;
            const id =
              input?.id ??
              input?.observationId ??
              input?.traceId ??
              input?.scoreId ??
              input?.projectId;
            const params = { ...input };
            delete params.id;
            delete params.observationId;
            delete params.traceId;
            delete params.scoreId;
            return qc.prefetchQuery({
              queryKey: fullKey,
              queryFn: () => {
                if (id) {
                  return rustGet(
                    `/api/${resource}/${encodeURIComponent(id)}`,
                    params,
                  );
                }
                return rustGet(`/api/${resource}`, params);
              },
              staleTime: opts?.staleTime,
            });
          },
        };
      }
      const resourceUtilsCache: Record<string, any> = {};
      const baseUtils = {
        invalidate: (queryKey?: any[]) => qc.invalidateQueries({ queryKey }),
        refetch: (queryKey?: any[]) => qc.refetchQueries({ queryKey }),
        getQueryData: (queryKey: any[]) => qc.getQueryData(queryKey),
        setQueryData: (queryKey: any[], data: any) => qc.setQueryData(queryKey, data),
      };
      return new Proxy(baseUtils, {
        get(target: any, resource: string) {
          if (resource in target) return target[resource];
          if (typeof resource !== "string") return undefined;
          if (resource === "then") return undefined;
          if (!resourceUtilsCache[resource]) {
            resourceUtilsCache[resource] = new Proxy(
              {
                // Allow utils.<resource>.invalidate() which is used by legacy code
                invalidate: () => qc.invalidateQueries({ queryKey: [resource] }),
                refetch: () => qc.refetchQueries({ queryKey: [resource] }),
              },
              {
                get(target: any, procedure: string) {
                  if (typeof procedure !== "string") return undefined;
                  // If the key exists on target (e.g. invalidate), return it directly
                  if (target[procedure] !== undefined) return target[procedure];
                  return buildProcUtils(resource, procedure);
                },
              },
            );
          }
          return resourceUtilsCache[resource];
        },
      });
    },
  },
  {
    get(target: any, resource: string) {
      if (resource in target) return target[resource];
      if (typeof resource !== "string") return undefined;
      if (resource === "then") return undefined;
      if (resource === Symbol.toPrimitive) return undefined;

      if (!resourceCache[resource]) {
        resourceCache[resource] = createResourceProxy(resource);
      }
      return resourceCache[resource];
    },
  },
);

// Pre-populate cache for known real resources with real procedures
for (const resource of Object.keys(REAL_RESOURCES)) {
  resourceCache[resource] = createResourceProxy(resource);
}

// Special procedures that need custom endpoints
// traces.byIdWithObservationsAndScores → GET /api/traces/{traceId}/full
(function() {
  const proc = {
    useQuery(input: any, opts?: any) {
      const traceId = input?.traceId || input?.id;
      const params: Record<string, any> = {};
      if (input?.projectId) params.project_id = input.projectId;
      if (input?.project_id) params.project_id = input.project_id;
      return useQuery({
        ...opts,
        queryKey: ["traces", "byIdWithObservationsAndScores", input],
        queryFn: async () => {
          const nested = await rustGet(`/api/traces/${encodeURIComponent(traceId)}/full`, params) as any;
          // Rust API returns { trace: {...}, observations: [...], scores: [...], corrections: [] }
          // Flatten to match tRPC format: { ...traceFields, observations, scores, corrections }
          if (nested?.trace) {
            const { trace, ...rest } = nested;
            return { ...trace, ...rest };
          }
          return nested;
        },
        enabled: !!traceId && (opts?.enabled !== false),
      } as any);
    },
    useInfiniteQuery(input: any, opts?: any) { return proc.useQuery(input, opts); },
    useSuspenseQuery(input: any, opts?: any) { return proc.useQuery(input, opts); },
  };
  resourceCache["traces"] = resourceCache["traces"] || createResourceProxy("traces");
  resourceCache["traces"].byIdWithObservationsAndScores = proc;
})();
// projects.create → POST /api/projects?organization_id=... with { name }
(function() {
  const proc = mutationProc(
    "POST", "/api/projects", "projects", "create",
    undefined,
    (input: any) => ({ organization_id: input?.orgId }),
    (input: any) => ({ name: input?.name }),
  );
  resourceCache["projects"] = resourceCache["projects"] || createResourceProxy("projects");
  resourceCache["projects"].create = proc;
})();
// apiKeys.create → POST /api/api-keys, apiKeys.all → GET /api/api-keys, apiKeys.delete → DELETE /api/api-keys/:id
(function() {
  const createProc = mutationProc(
    "POST", "/api/api-keys", "apiKeys", "create",
    undefined,
    undefined,
    (input: any) => ({ project_id: input?.projectId, note: input?.note }),
  );
  const listProc = queryProc("/api/api-keys", "apiKeys", "all", (input: any) => {
    const params: Record<string, any> = {};
    if (input?.project_id) params.project_id = input.project_id;
    if (input?.projectId) params.project_id = input.projectId;
    return params;
  });
  const deleteProc = mutationProc(
    "DELETE", "/api/api-keys", "apiKeys", "delete",
    (input: any) => `/api/api-keys/${encodeURIComponent(input?.id || input)}`,
  );
  resourceCache["apiKeys"] = resourceCache["apiKeys"] || createResourceProxy("apiKeys");
  resourceCache["apiKeys"].create = createProc;
  resourceCache["apiKeys"].all = listProc;
  resourceCache["apiKeys"].delete = deleteProc;
// projectApiKeys.create → POST /api/api-keys, projectApiKeys.byProjectId → GET /api/api-keys?project_id=xxx, projectApiKeys.delete → DELETE /api/api-keys/:id, projectApiKeys.updateNote → PUT /api/api-keys/:id
(function() {
  const createProc = mutationProc(
    "POST", "/api/api-keys", "projectApiKeys", "create",
    undefined,
    undefined,
    (input: any) => ({ project_id: input?.projectId, note: input?.note }),
  );
  const listProc = queryProc("/api/api-keys", "projectApiKeys", "byProjectId", (input: any) => {
    const params: Record<string, any> = {};
    if (input?.projectId) params.project_id = input.projectId;
    if (input?.project_id) params.project_id = input.project_id;
    return params;
  });
  const deleteProc = mutationProc(
    "DELETE", "/api/api-keys", "projectApiKeys", "delete",
    (input: any) => `/api/api-keys/${encodeURIComponent(input?.id || input)}`,
  );
  const updateNoteProc = mutationProc(
    "PUT", "/api/api-keys", "projectApiKeys", "updateNote",
    (input: any) => `/api/api-keys/${encodeURIComponent(input?.id || input)}`,
    undefined,
    (input: any) => ({ note: input?.note }),
  );
  resourceCache["projectApiKeys"] = resourceCache["projectApiKeys"] || createResourceProxy("projectApiKeys");
  resourceCache["projectApiKeys"].create = createProc;
  resourceCache["projectApiKeys"].byProjectId = listProc;
  resourceCache["projectApiKeys"].delete = deleteProc;
  resourceCache["projectApiKeys"].updateNote = updateNoteProc;
})();

})();



// ============================================================================
// Direct API client (non-hook usage)
// ============================================================================

export const directApi: any = new Proxy(
  {},
  {
    get(_target: any, resource: string) {
      return createResourceProxy(resource);
    },
  },
);

// ============================================================================
// Type exports (compatibility)
// ============================================================================

export type RouterInputs = { [resource: string]: { [procedure: string]: any } };
export type RouterOutputs = { [resource: string]: { [procedure: string]: any } };

export const getPathnameWithoutBasePath = () => {
  return window.location.pathname;
};
