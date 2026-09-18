/**
 * PG-Only: Project sentinel page.
 * Redirects to the last-visited project or first available project.
 * Uses Rust auth session (JWT cookie) instead of NextAuth.
 */
import {
  type GetServerSideProps,
  type GetServerSidePropsContext,
  type GetServerSidePropsResult,
} from "next";

/** RedirectToFirstProject renders nothing; all logic runs in getServerSideProps. */
const RedirectToFirstProject = () => null;
export default RedirectToFirstProject;

/** getServerSideProps resolves the project sentinel. */
export const getServerSideProps: GetServerSideProps = async (ctx) => {
  const sCtx = parseSentinelRequest(ctx);

  const crossRegion = crossRegionRedirect(sCtx);
  if (crossRegion) return crossRegion;

  const signIn = await signInRedirect(sCtx);
  if (signIn) return signIn;

  return projectRedirect(sCtx);
};

// ============================================================================
// Cookie utilities (inline, was in @/src/server/utils/cookies)
// ============================================================================

interface ProjectCookie {
  projectId: string;
  origin: string;
}

function readProjectCookie(cookies: Record<string, string | undefined>): ProjectCookie | null {
  const raw = cookies["langfuse_project"] ?? cookies["langfuse-project"];
  if (!raw) return null;
  try {
    return JSON.parse(raw);
  } catch {
    return null;
  }
}

function getRequestOrigin(req: GetServerSidePropsContext["req"]): string | null {
  const host = req.headers.host;
  const proto = req.headers["x-forwarded-proto"] ?? "http";
  if (!host) return null;
  return `${proto}://${host}`;
}

// ============================================================================
// Session check (replaces NextAuth getServerAuthSession)
// ============================================================================

async function getSession(req: GetServerSidePropsContext["req"]): Promise<{
  user: {
    id: string;
    email?: string;
    name?: string;
    organizations: Array<{
      id: string;
      name: string;
      role: string;
      projects: Array<{ id: string; name: string; role: string }>;
    }>;
  } | null;
} | null> {
  try {
    const cookie = req.headers.cookie;
    // Call Rust /api/auth/session to verify JWT
    const origin = "http://localhost:8010";
    const res = await fetch(`${origin}/api/auth/session`, {
      headers: cookie ? { Cookie: cookie } : {},
    });
    if (!res.ok) return null;
    const data = await res.json();
    return data?.user ? data : null;
  } catch {
    return null;
  }
}

// ============================================================================
// Sentinel logic
// ============================================================================

interface SentinelContext {
  cookie: ProjectCookie | null;
  origin: string | null;
  resolvedUrl: string;
  getSession: () => Promise<Awaited<ReturnType<typeof getSession>>>;
}

const parseSentinelRequest = (ctx: GetServerSidePropsContext): SentinelContext => {
  let session: ReturnType<typeof getSession> | undefined;
  return {
    cookie: readProjectCookie(ctx.req.cookies ?? {}),
    origin: getRequestOrigin(ctx.req),
    resolvedUrl: ctx.resolvedUrl,
    getSession: () => (session ??= getSession(ctx.req)),
  };
};

const crossRegionRedirect = (req: SentinelContext) => {
  const { cookie, origin, resolvedUrl } = req;
  if (
    cookie &&
    origin &&
    cookie.origin !== origin &&
    sameRegistrableDomain(cookie.origin, origin)
  ) {
    return redirect(`${cookie.origin}${resolvedUrl}`);
  }
};

const signInRedirect = async ({ getSession, resolvedUrl }: SentinelContext) => {
  const session = await getSession();
  if (!session?.user)
    return redirect(
      `/auth/sign-in?callbackUrl=${encodeURIComponent(resolvedUrl)}`,
    );
};

const projectRedirect = async ({
  cookie,
  origin,
  resolvedUrl,
  getSession,
}: SentinelContext) => {
  const session = await getSession();
  const rest = resolvedUrl.slice(sentinelPathPrefix.length);
  const projects =
    session?.user?.organizations.flatMap((org) => org.projects) ?? [];

  if (cookie && origin && cookie.origin === origin) {
    if (projects.some((p) => p.id === cookie.projectId)) {
      return redirect(`/project/${cookie.projectId}${rest}`);
    }
  }

  const firstProjectId = projects.at(0)?.id;
  if (!firstProjectId) return redirect("/");

  return redirect(`/project/${firstProjectId}${rest}`);
};

const redirect = (destination: string): GetServerSidePropsResult<never> => ({
  redirect: { destination, permanent: false },
});

const sameRegistrableDomain = (originA: string, originB: string): boolean => {
  const registrableDomain = (origin: string): string | null => {
    try {
      return new URL(origin).hostname.split(".").slice(-2).join(".");
    } catch {
      return null;
    }
  };
  const domainA = registrableDomain(originA);
  const domainB = registrableDomain(originB);
  return domainA !== null && domainA === domainB;
};

const sentinelPathPrefix = "/project/~";
