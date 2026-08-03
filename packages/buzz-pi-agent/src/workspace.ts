import { realpathSync, statSync } from "node:fs";
import { resolve } from "node:path";

export interface WorkspaceIdentity {
  requestedPath: string;
  canonicalPath: string;
  device: string;
  inode: string;
}

/**
 * Resolve an existing workspace to one stable target before any persistence,
 * trust decision, session creation, or resource discovery uses it.
 */
export function captureWorkspaceIdentity(
  requestedPath: string,
  expectedCanonicalPath?: string,
): WorkspaceIdentity {
  const resolvedRequestedPath = resolve(requestedPath);
  let canonicalPath: string;
  let stats: ReturnType<typeof statSync>;
  try {
    canonicalPath = realpathSync.native(resolvedRequestedPath);
    stats = statSync(canonicalPath, { bigint: true });
    const verifiedCanonicalPath = realpathSync.native(resolvedRequestedPath);
    const verifiedStats = statSync(verifiedCanonicalPath, { bigint: true });
    if (
      verifiedCanonicalPath !== canonicalPath ||
      verifiedStats.dev !== stats.dev ||
      verifiedStats.ino !== stats.ino
    ) {
      throw workspaceError("changed targets while its identity was captured");
    }
  } catch (error) {
    if (
      error instanceof Error &&
      error.message.startsWith("BUZZ_PI_WORKSPACE_CHANGED:")
    ) {
      throw error;
    }
    throw workspaceError("is unavailable", error);
  }
  if (!stats.isDirectory()) {
    throw workspaceError("is not a directory");
  }
  if (
    expectedCanonicalPath !== undefined &&
    canonicalPath !== expectedCanonicalPath
  ) {
    throw workspaceError("changed targets before Pi session creation");
  }
  return {
    requestedPath: resolvedRequestedPath,
    canonicalPath,
    device: stats.dev.toString(),
    inode: stats.ino.toString(),
  };
}

/** Fail closed if a symlink target or the workspace directory inode changed. */
export function assertWorkspaceIdentity(identity: WorkspaceIdentity): void {
  const current = captureWorkspaceIdentity(
    identity.requestedPath,
    identity.canonicalPath,
  );
  if (current.device !== identity.device || current.inode !== identity.inode) {
    throw workspaceError("was replaced while the Pi session was active");
  }
}

function workspaceError(message: string, cause?: unknown): Error {
  return new Error(`BUZZ_PI_WORKSPACE_CHANGED: workspace ${message}`, {
    ...(cause === undefined ? {} : { cause }),
  });
}
