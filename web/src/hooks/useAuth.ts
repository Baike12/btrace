/**
 * useAuth / useSession — session access for the console.
 *
 * Authentication is real: `signIn` posts credentials to `/api/auth/login`,
 * which verifies the bcrypt hash in `users` and answers with an HttpOnly
 * session cookie. `fetchSession` then reads `/api/auth/session`, which resolves
 * that cookie to `{ user, organizations }` or `{}` when nobody is signed in.
 *
 * There is deliberately **no** hard-coded fallback user. An earlier revision
 * returned a fixed session whenever the API was unreachable (or returned an
 * unexpected payload), which meant a request from someone with no credentials
 * at all still produced an "authenticated" console backed by real data. A
 * failed fetch now means unauthenticated, and the layout sends the browser to
 * the sign-in page.
 */
import { useState, useEffect, useCallback } from "react";
import React from "react";

// ============================================================================
// Types — NextAuth compatible
// ============================================================================

export interface AuthProject {
  id: string;
  name: string;
  role: string;
}

export interface AuthOrganization {
  id: string;
  name: string;
  role: string;
  projects: AuthProject[];
}

export interface AuthUser {
  id: string;
  email?: string | null;
  name?: string | null;
  image?: string | null;
  organizations?: AuthOrganization[];
  featureFlags?: string[] | Record<string, boolean>;
  admin?: boolean;
  v4BetaEnabled?: boolean;
  canCreateOrganizations?: boolean;
  cloudRegion?: string;
}

export interface Session {
  user: AuthUser;
  environment?: {
    selfHostedInstancePlan?: string | null;
  };
  expires: string;
}

export interface SessionContextValue {
  data: Session | null;
  status: "authenticated" | "unauthenticated" | "loading";
  update: (data?: any) => Promise<Session | null>;
}

export type { SessionContextValue as SessionContextValue };

export interface User {
  id: string;
  email?: string | null;
  name?: string | null;
  image?: string | null;
  organizations?: AuthOrganization[];
  featureFlags?: string[] | Record<string, boolean>;
  admin?: boolean;
  [key: string]: unknown;
}

export interface Provider {
  id: string;
  name: string;
  type: string;
}

// ============================================================================
// Session fetch
// ============================================================================

/** How long a fetched session is reused before the next call revalidates it. */
const SESSION_CACHE_MS = 5 * 60 * 1000;

/**
 * Read the current session from the backend.
 *
 * `null` means "not signed in" — either the endpoint answered `{}` (NextAuth's
 * shape for no session), or the request failed. Both are the same thing from
 * the caller's point of view, and neither may be papered over with a synthetic
 * user.
 */
async function fetchSession(): Promise<Session | null> {
  try {
    const res = await fetch("/api/auth/session", {
      credentials: "same-origin",
      headers: { Accept: "application/json" },
    });
    if (!res.ok) return null;

    const data = await res.json();
    if (!data?.user?.id) return null;
    return data as Session;
  } catch {
    // Network failure or a non-JSON body. Unauthenticated, not "logged in as
    // whoever the fallback constant named".
    return null;
  }
}

// ============================================================================
// Module-level session cache
// ============================================================================

/** `undefined` = not fetched yet, which the hook reports as `loading`. */
let cachedSession: Session | null | undefined = undefined;
let cacheExpiry = 0;
let listeners: Array<() => void> = [];

function notifyListeners() {
  listeners.forEach((fn) => fn());
}

function store(session: Session | null) {
  cachedSession = session;
  cacheExpiry = Date.now() + SESSION_CACHE_MS;
  notifyListeners();
}

/** Drop the cached session and tell every subscriber to re-read it. */
function invalidate() {
  cachedSession = undefined;
  cacheExpiry = 0;
  notifyListeners();
}

// ============================================================================
// useSession
// ============================================================================

export function useSession(): SessionContextValue {
  const [, setTick] = useState(0);

  const refresh = useCallback(async () => {
    if (cachedSession !== undefined && Date.now() < cacheExpiry) {
      return cachedSession;
    }
    const session = await fetchSession();
    store(session);
    return session;
  }, []);

  useEffect(() => {
    const listener = () => setTick((t) => t + 1);
    listeners.push(listener);
    return () => {
      listeners = listeners.filter((l) => l !== listener);
    };
  }, []);

  useEffect(() => {
    if (cachedSession === undefined) {
      refresh();
    }
  }, [refresh]);

  const update = useCallback(async (_data?: any): Promise<Session | null> => {
    const session = await fetchSession();
    store(session);
    return session;
  }, []);

  if (cachedSession === undefined) {
    return { data: null, status: "loading", update };
  }
  if (cachedSession === null) {
    return { data: null, status: "unauthenticated", update };
  }
  return { data: cachedSession, status: "authenticated", update };
}

// ============================================================================
// getSession
// ============================================================================

export async function getSession(): Promise<Session | null> {
  const session = await fetchSession();
  store(session);
  return session;
}

// ============================================================================
// signIn
// ============================================================================

/**
 * Sign in with an email and password.
 *
 * `options.password` is required — the only credential this fork accepts.
 * Passing just a provider name (as the SSO pages do) cannot succeed, and says
 * so instead of resolving `{ ok: true }` without a session behind it.
 */
export async function signIn(
  email?: string,
  options?: { password?: string; [key: string]: unknown },
  _authorizationParams?: unknown,
): Promise<{ ok: boolean; error?: string }> {
  const password = options?.password;

  if (!email || typeof password !== "string" || password.length === 0) {
    return { ok: false, error: "Email and password are required" };
  }

  try {
    const res = await fetch("/api/auth/login", {
      method: "POST",
      credentials: "same-origin",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ email, password }),
    });

    if (!res.ok) {
      return { ok: false, error: await errorMessage(res, "Invalid email or password") };
    }

    // The response body carries the session's user, but re-reading
    // `/api/auth/session` is what proves the browser actually stored the
    // cookie — a login whose cookie was dropped (wrong `Secure` setting, a
    // blocked `Set-Cookie`) would otherwise look like a success and bounce the
    // user straight back to the sign-in page.
    invalidate();
    const session = await fetchSession();
    store(session);

    return session ? { ok: true } : { ok: false, error: "Sign-in did not persist. Check that cookies are enabled." };
  } catch {
    return { ok: false, error: "Network error" };
  }
}

// ============================================================================
// signOut
// ============================================================================

export async function signOut(): Promise<void> {
  try {
    await fetch("/api/auth/logout", {
      method: "POST",
      credentials: "same-origin",
    });
  } catch {
    // The cookie may still be live server-side, but the local view must not
    // keep showing data the user asked to leave. The next navigation re-reads
    // the session and will still be signed out if the server did clear it.
  }
  cachedSession = null;
  cacheExpiry = Date.now() + SESSION_CACHE_MS;
  notifyListeners();
}

// ============================================================================
// signUp
// ============================================================================

/**
 * Create an account. The backend also provisions a personal organization and a
 * first project, so the new user lands in a usable console rather than an empty
 * project list.
 */
export async function signUp(
  email: string,
  name: string,
  password: string,
): Promise<{ ok: boolean; error?: string }> {
  try {
    const res = await fetch("/api/auth/signup", {
      method: "POST",
      credentials: "same-origin",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ email, name, password }),
    });

    if (!res.ok) {
      return { ok: false, error: await errorMessage(res, "Sign up failed") };
    }

    invalidate();
    const session = await fetchSession();
    store(session);

    return session ? { ok: true } : { ok: false, error: "Sign-up did not persist. Check that cookies are enabled." };
  } catch {
    return { ok: false, error: "Network error" };
  }
}

/**
 * Pull the human-readable reason out of a failed auth response.
 *
 * The auth routes answer with `{ message, error }`. Falling back to the plain
 * body keeps a non-JSON error (a proxy's HTML error page, say) from surfacing
 * as "[object Object]" in the form.
 */
async function errorMessage(res: Response, fallback: string): Promise<string> {
  try {
    const body = await res.json();
    if (typeof body?.message === "string" && body.message.length > 0) {
      return body.message;
    }
  } catch {
    // not JSON
  }
  return fallback;
}

// ============================================================================
// SessionProvider
// ============================================================================

export function SessionProvider({
  children,
  session: _initialSession,
  refetchOnWindowFocus = true,
  refetchInterval,
  basePath: _basePath,
}: {
  children: React.ReactNode;
  session?: Session | null;
  refetchOnWindowFocus?: boolean;
  refetchInterval?: number;
  basePath?: string;
}) {
  // Fetch on mount. The cache deliberately starts empty — seeding it with a
  // placeholder session would render protected pages (and fire their data
  // requests) before the server has said whether this browser is signed in.
  useEffect(() => {
    let cancelled = false;
    fetchSession().then((s) => {
      if (!cancelled) store(s);
    });
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    if (refetchOnWindowFocus) {
      const onFocus = () => {
        fetchSession().then(store);
      };
      window.addEventListener("focus", onFocus);
      return () => window.removeEventListener("focus", onFocus);
    }
  }, [refetchOnWindowFocus]);

  useEffect(() => {
    if (refetchInterval && refetchInterval > 0) {
      const timer = setInterval(() => {
        fetchSession().then(store);
      }, refetchInterval * 1000);
      return () => clearInterval(timer);
    }
  }, [refetchInterval]);

  return React.createElement(React.Fragment, null, children);
}
