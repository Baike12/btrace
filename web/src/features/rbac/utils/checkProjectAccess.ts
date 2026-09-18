import {
  projectRoleAccessRights,
  type ProjectScope,
} from "@/src/features/rbac/constants/projectAccessRights";
import { type Role } from "@langfuse/shared";
// PG-only: TRPCError replaced with generic Error
class TRPCError extends Error { constructor(opts: { code: string; message: string }) { super(opts.message); this.name = opts.code; } }
import { type Session } from "@/src/hooks/useAuth";
import { useSession } from "@/src/hooks/useAuth";
import { hasOwnRole } from "./hasOwnRole";

type HasProjectAccessParams = (
  | {
      role: Role;
      scope: ProjectScope;
      admin?: boolean; // prop user.admin
    }
  | {
      session: null | Session;
      projectId: string;
      scope: ProjectScope;
    }
) & { forbiddenErrorMessage?: string };

/**
 * Check if user has access to the given scope, for use in TRPC resolvers
 * @throws TRPCError("FORBIDDEN") if user does not have access
 */
export const throwIfNoProjectAccess = (p: HasProjectAccessParams) => {
  if (!hasProjectAccess(p))
    throw new TRPCError({
      code: "FORBIDDEN",
      message:
        p.forbiddenErrorMessage ??
        "User does not have access to this resource or action",
    });
};

/**
 * React hook to check if user has access to the given scope
 * @returns true if user has access, false otherwise or while loading
 */
export const useHasProjectAccess = (p: {
  projectId: string | undefined;
  scope: ProjectScope;
}) => {
  const { scope, projectId } = p;
  const session = useSession();

  if (session.data?.user?.admin) return true;
  if (!projectId) return false;

  return hasProjectAccess({ session: session.data, scope, projectId });
};

// For use in UI components as function, if session is already available
export function hasProjectAccess(p: HasProjectAccessParams): boolean {
  const isAdmin = hasOwnRole(p) ? p.admin : p.session?.user?.admin;
  if (isAdmin) return true;

  const projectRole: Role | undefined = hasOwnRole(p)
    ? p.role
    : p.session?.user?.organizations
        .flatMap((org) => org.projects)
        .find((project) => project.id === p.projectId)?.role;
  if (projectRole === undefined) return false;

  return projectRoleAccessRights[projectRole].includes(p.scope);
}
