/**
 * Type-safe API client for Rust backend.
 * Generated from OpenAPI spec → api-types.ts
 *
 * Usage:
 *   import { apiClient } from "@/src/lib/api-client";
 *
 *   // Public endpoint (no auth)
 *   const { data, error } = await apiClient.POST("/api/auth/login", { body: { email, password } });
 *
 *   // Private endpoint (JWT required)
 *   const token = "eyJ...";
 *   const { data } = await apiClient.GET("/api/traces", {
 *     params: { query: { project_id: "xxx", limit: 50 } },
 *     headers: { Authorization: `Bearer ${token}` },
 *   });
 */

import type { paths } from "./api-types";

type ApiPaths = paths;

const BASE_URL = (typeof window !== "undefined")
  ? window.location.origin
  : process.env.NEXT_PUBLIC_API_URL || "http://localhost:3000";

type HttpMethod = "GET" | "POST" | "PUT" | "DELETE" | "PATCH";

type PathParams<T extends string> =
  T extends `${infer _Start}{${infer Param}}${infer Rest}`
    ? { [K in Param | keyof PathParams<Rest>]: string }
    : Record<string, never>;

type ExtractQuery<Op> =
  Op extends { parameters: { query?: infer Q } } ? (Q extends Record<string, unknown> ? Q : never) : never;

type ExtractBody<Op> =
  Op extends { requestBody?: { content: { "application/json": infer B } } } ? B : never;

type ExtractResponse<Op, Status extends number> =
  Op extends { responses: { [K in Status]: { content: { "application/json": infer R } } } } ? R : never;

type ApiResponse<Data> =
  | { data: Data; error: null }
  | { data: null; error: string };

type FetchOptions = Omit<RequestInit, "body"> & {
  params?: { query?: Record<string, string | number | undefined>; path?: Record<string, string> };
  body?: unknown;
};

function buildUrl(path: string, opts?: FetchOptions): string {
  let url = `${BASE_URL}${path}`;

  // Replace path params: /api/traces/{trace_id} → /api/traces/123
  if (opts?.params?.path) {
    for (const [key, value] of Object.entries(opts.params.path)) {
      url = url.replace(`{${key}}`, encodeURIComponent(value));
    }
  }

  // Append query params
  if (opts?.params?.query) {
    const params = new URLSearchParams();
    for (const [key, value] of Object.entries(opts.params.query)) {
      if (value !== undefined && value !== null) {
        params.append(key, String(value));
      }
    }
    const qs = params.toString();
    if (qs) url += `?${qs}`;
  }

  return url;
}

async function request<T>(
  method: HttpMethod,
  path: string,
  opts?: FetchOptions,
): Promise<ApiResponse<T>> {
  try {
    const url = buildUrl(path, opts);
    const headers: Record<string, string> = {
      "Content-Type": "application/json",
      ...((opts?.headers as Record<string, string>) || {}),
    };

    const res = await fetch(url, {
      method,
      headers,
      credentials: "include", // for cookie-based auth
      body: opts?.body ? JSON.stringify(opts.body) : undefined,
    });

    if (!res.ok) {
      if (res.status === 401) return { data: null, error: "Unauthorized" };
      if (res.status === 404) return { data: null, error: "Not found" };
      const text = await res.text().catch(() => "Unknown error");
      return { data: null, error: text };
    }

    if (res.status === 204 || res.headers.get("content-length") === "0") {
      return { data: null as unknown as T, error: null };
    }

    const json = await res.json();
    return { data: json as T, error: null };
  } catch (e) {
    return { data: null, error: e instanceof Error ? e.message : "Network error" };
  }
}

export const apiClient = {
  GET: <P extends keyof ApiPaths>(path: P, opts?: FetchOptions) =>
    request<unknown>("GET", path as string, opts),

  POST: <P extends keyof ApiPaths>(path: P, opts?: FetchOptions) =>
    request<unknown>("POST", path as string, opts),

  PUT: <P extends keyof ApiPaths>(path: P, opts?: FetchOptions) =>
    request<unknown>("PUT", path as string, opts),

  DELETE: <P extends keyof ApiPaths>(path: P, opts?: FetchOptions) =>
    request<unknown>("DELETE", path as string, opts),

  PATCH: <P extends keyof ApiPaths>(path: P, opts?: FetchOptions) =>
    request<unknown>("PATCH", path as string, opts),
};

/**
 * Create an auth-aware client with a JWT token.
 * Use this in SSR pages or client-side after Rust login.
 */
export function createAuthClient(token: string) {
  const authHeaders = { Authorization: `Bearer ${token}` };
  return {
    GET: <P extends keyof ApiPaths>(path: P, opts?: Omit<FetchOptions, "headers">) =>
      apiClient.GET(path, { ...opts, headers: authHeaders }),
    POST: <P extends keyof ApiPaths>(path: P, opts?: Omit<FetchOptions, "headers">) =>
      apiClient.POST(path, { ...opts, headers: authHeaders }),
    PUT: <P extends keyof ApiPaths>(path: P, opts?: Omit<FetchOptions, "headers">) =>
      apiClient.PUT(path, { ...opts, headers: authHeaders }),
    DELETE: <P extends keyof ApiPaths>(path: P, opts?: Omit<FetchOptions, "headers">) =>
      apiClient.DELETE(path, { ...opts, headers: authHeaders }),
  };
}
