/**
 * Rust REST API 客户端
 *
 * 用于 Next.js 服务端 (tRPC router) 和浏览器端调用 Rust 后端 API。
 * 替代 Prisma 直接访问数据库，使所有数据流经 Rust 后端。
 */

const RUST_API_URL = process.env.RUST_API_URL || "http://localhost:8080";

interface RustResponse<T = any> {
  data: T | null;
  meta: { cursor: string | null; has_more: boolean; total: number | null } | null;
  error: string | null;
}

/**
 * 从 Next.js request 中提取认证 cookie，转发到 Rust
 */
function getAuthHeaders(req?: { headers: { cookie?: string }; cookies?: Record<string, string> }): Record<string, string> {
  const headers: Record<string, string> = { "Content-Type": "application/json" };
  if (req?.headers?.cookie) {
    headers["Cookie"] = req.headers.cookie;
  }
  return headers;
}

/**
 * 调用 Rust REST API
 */
async function rustFetch<T = any>(
  path: string,
  opts?: {
    method?: string;
    params?: Record<string, string | number | undefined>;
    body?: unknown;
    req?: { headers: { cookie?: string } };
  },
): Promise<{ data: T | null; error: string | null }> {
  try {
    let url = `${RUST_API_URL}${path}`;
    if (opts?.params) {
      const searchParams = new URLSearchParams();
      for (const [k, v] of Object.entries(opts.params)) {
        if (v !== undefined && v !== null) searchParams.append(k, String(v));
      }
      const qs = searchParams.toString();
      if (qs) url += `?${qs}`;
    }

    const headers = getAuthHeaders(opts?.req as any);
    const res = await fetch(url, {
      method: opts?.method || "GET",
      headers,
      body: opts?.body ? JSON.stringify(opts.body) : undefined,
    });

    if (res.status === 401) return { data: null, error: "Unauthorized" };
    if (res.status === 404) return { data: null, error: "Not found" };
    if (!res.ok) {
      const text = await res.text().catch(() => "Error");
      return { data: null, error: text };
    }

    const json: RustResponse<T> = await res.json();
    if (json.error) return { data: null, error: json.error };
    return { data: json.data, error: null };
  } catch (e) {
    return { data: null, error: e instanceof Error ? e.message : "Network error" };
  }
}

/**
 * Rust API — 按资源分组的类型安全客户端
 */
export const rustApi = {
  traces: {
    list: (params: { project_id: string; limit?: number; cursor?: string }) =>
      rustFetch<any>("/api/traces", { params: { project_id: params.project_id, limit: params.limit, cursor: params.cursor } }),
    get: (projectId: string, traceId: string) =>
      rustFetch<any>(`/api/traces/${traceId}`, { params: { project_id: projectId } }),
  },
  observations: {
    list: (params: { project_id: string; trace_id?: string; limit?: number }) =>
      rustFetch<any>("/api/observations", { params: params as any }),
  },
  scores: {
    list: (params: { project_id: string; trace_id?: string; limit?: number }) =>
      rustFetch<any>("/api/scores", { params: params as any }),
  },
  sessions: {
    list: (params: { project_id: string; limit?: number }) =>
      rustFetch<any>("/api/sessions", { params: params as any }),
  },
  datasets: {
    list: (params: { project_id: string; limit?: number }) =>
      rustFetch<any>("/api/datasets", { params: params as any }),
  },
  prompts: {
    list: (params: { project_id: string; limit?: number }) =>
      rustFetch<any>("/api/prompts", { params: params as any }),
  },
  projects: {
    list: () => rustFetch<any>("/api/projects"),
  },
  organizations: {
    list: () => rustFetch<any>("/api/organizations"),
  },
  models: {
    list: (params: { project_id: string; limit?: number }) =>
      rustFetch<any>("/api/models", { params: params as any }),
  },
  evals: {
    list: (params: { project_id: string; limit?: number }) =>
      rustFetch<any>("/api/evals", { params: params as any }),
  },
  dashboards: {
    list: (params: { project_id: string; limit?: number }) =>
      rustFetch<any>("/api/dashboards", { params: params as any }),
  },
  comments: {
    list: (params: { project_id: string; limit?: number; object_type?: string; object_id?: string }) =>
      rustFetch<any>("/api/comments", { params: params as any }),
  },
  media: {
    list: (params: { project_id: string; limit?: number }) =>
      rustFetch<any>("/api/media", { params: params as any }),
  },
  automations: {
    list: (params: { project_id: string; limit?: number }) =>
      rustFetch<any>("/api/automations", { params: params as any }),
  },
  monitors: {
    list: (params: { project_id: string; limit?: number }) =>
      rustFetch<any>("/api/monitors", { params: params as any }),
  },
  users: {
    list: (params?: { limit?: number }) =>
      rustFetch<any>("/api/users", { params: params as any }),
  },
  auth: {
    session: () => rustFetch<any>("/api/auth/session"),
    login: (email: string, password: string) =>
      rustFetch<any>("/api/auth/login", { method: "POST", body: { email, password } }),
    signup: (email: string, name: string, password: string) =>
      rustFetch<any>("/api/auth/signup", { method: "POST", body: { email, name, password } }),
    logout: () => rustFetch<any>("/api/auth/logout", { method: "POST" }),
  },
};
