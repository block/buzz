import { createHash, randomUUID } from "node:crypto";
import { execFileSync } from "node:child_process";
import { readFileSync, readlinkSync } from "node:fs";
import {
  chmod,
  lstat,
  mkdir,
  open,
  opendir,
  readFile,
  realpath,
  rename,
  rm,
  rmdir,
  stat,
  unlink,
  utimes,
  writeFile,
} from "node:fs/promises";
import { hostname } from "node:os";
import { basename, isAbsolute, join, relative, resolve, sep } from "node:path";
import { getAgentDir } from "@earendil-works/pi-coding-agent";
import type { AdapterConfig } from "./config.js";
import type {
  BuzzSessionEvent,
  ConversationMapping,
  Logger,
  PendingBuzzSessionEvent,
} from "./types.js";

interface ConversationManifest {
  version: 1;
  conversations: Record<string, ConversationMapping>;
  resetTombstones: Record<string, ResetTombstone>;
}

interface ResetTombstoneBase {
  conversationId: string;
  previousPiSessionId?: string;
  resetToken?: string;
  createdAt: string;
}

type ResetTombstone =
  | (ResetTombstoneBase & { status: "pending" })
  | (ResetTombstoneBase & {
      status: "retained";
      installedPiSessionId: string;
      consumedAt: string;
    });

interface PruneCandidate {
  mapping: ConversationMapping;
  reason: "ttl" | "capacity";
}

export interface ResolveConversationResult {
  mapping: ConversationMapping;
  lifecycleGeneration: string;
  resumed: boolean;
  previousPiSessionId?: string;
  retiredSessionFile?: string;
  skipRelayHistory: boolean;
  refresh: () => Promise<boolean>;
  forget: () => Promise<string | undefined>;
  release: () => Promise<void>;
}

interface AcquiredStateLock {
  assertOwned: () => Promise<void>;
  release: () => Promise<void>;
}

interface StateLockGeneration {
  device: number;
  inode: number;
  birthtimeMs: number;
  mtimeMs: number;
  ownerRaw: string | undefined;
}

/** Host/process identity used to decide whether a PID liveness probe is safe. */
export interface LeaseProcessIdentity {
  hostId: string;
  bootId?: string;
  pidProbeSafe: boolean;
}

export interface CommitConversationResetResult {
  alreadyCommitted: boolean;
  disposeLiveSession: boolean;
  retiredSessionFile?: string;
}

const MAX_MANIFEST_BYTES = 16 * 1024 * 1024;
const MAX_MANIFEST_ENTRIES = 100_000;
const MAX_CWD_CHARACTERS = 2_048;
const MAX_SESSION_PATH_CHARACTERS = 4_096;
const LOCK_OWNER_WRITE_GRACE_MS = 2_000;
const LOCK_FOREIGN_STALE_MS = 2 * 60_000;
const LOCK_HEARTBEAT_MS = 30_000;
const INITIALIZED_MARKER = ".buzz-pi-state-v1";
const MAX_PENDING_SESSION_EVENT_BYTES = 64 * 1_024;
const EVENT_ID_PATTERN =
  /^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/u;
const EVENT_DIRECTORY_PATTERN = /^[0-9a-f]{64}$/u;
const LIFECYCLE_GENERATION_PATTERN = /^[0-9a-f]{64}$/u;
const SESSION_EVENT_TYPES = new Set([
  "compaction_completed",
  "compaction_failed",
  "context_status",
  "session_reset",
  "extensions_reloaded",
]);
const LOCAL_LEASE_IDENTITY = discoverLocalLeaseIdentity();

export class ConversationStore {
  readonly namespace: string;
  private readonly stateRoot: string;
  private readonly directory: string;
  private readonly manifestPath: string;
  private readonly initializedMarkerPath: string;
  private readonly locksDirectory: string;
  private readonly pendingEventsDirectory: string;
  private readonly leaseIdentity: LeaseProcessIdentity;
  private readonly allowedSessionRoots: readonly string[];

  constructor(
    private readonly config: AdapterConfig,
    private readonly logger: Logger,
    env: NodeJS.ProcessEnv = process.env,
    leaseIdentity: LeaseProcessIdentity = LOCAL_LEASE_IDENTITY,
  ) {
    this.leaseIdentity = leaseIdentity;
    this.namespace = deriveNamespace(env);
    this.stateRoot = resolve(config.stateDir);
    this.directory = resolve(this.stateRoot, this.namespace);
    if (!pathIsStrictlyWithin(this.directory, this.stateRoot)) {
      throw new Error("BUZZ_PI_NAMESPACE escapes BUZZ_PI_STATE_DIR");
    }
    this.manifestPath = join(this.directory, "conversations.json");
    this.initializedMarkerPath = join(this.directory, INITIALIZED_MARKER);
    this.locksDirectory = join(this.directory, "locks");
    this.pendingEventsDirectory = join(this.directory, "pending-events");
    // Pi-created sessions live in the first root. The adapter state root is
    // also allowed for controlled test/migration sessions, but arbitrary
    // absolute JSONL paths are never opened or deleted from the manifest.
    this.allowedSessionRoots = [
      resolve(join(getAgentDir(), "sessions")),
      this.stateRoot,
    ];
  }

  async initialize(): Promise<void> {
    await mkdir(this.stateRoot, { recursive: true, mode: 0o700 });
    await chmod(this.stateRoot, 0o700);
    await mkdir(this.directory, { recursive: true, mode: 0o700 });
    const namespaceMetadata = await lstat(this.directory);
    if (
      namespaceMetadata.isSymbolicLink() ||
      !namespaceMetadata.isDirectory()
    ) {
      throw new Error("Pi state namespace must be a real directory");
    }
    const [physicalRoot, physicalDirectory] = await Promise.all([
      realpath(this.stateRoot),
      realpath(this.directory),
    ]);
    if (!pathIsStrictlyWithin(physicalDirectory, physicalRoot)) {
      throw new Error("Pi state namespace resolves outside BUZZ_PI_STATE_DIR");
    }
    await mkdir(this.locksDirectory, { recursive: true, mode: 0o700 });
    await mkdir(this.pendingEventsDirectory, {
      recursive: true,
      mode: 0o700,
    });
    await chmod(this.directory, 0o700);
    await chmod(this.locksDirectory, 0o700);
    await chmod(this.pendingEventsDirectory, 0o700);
    const pendingEventsMetadata = await lstat(this.pendingEventsDirectory);
    if (
      pendingEventsMetadata.isSymbolicLink() ||
      !pendingEventsMetadata.isDirectory()
    ) {
      throw new Error("Pi pending-event state must be a real directory");
    }
    const physicalEventsDirectory = await realpath(this.pendingEventsDirectory);
    if (!pathIsStrictlyWithin(physicalEventsDirectory, physicalDirectory)) {
      throw new Error("Pi pending-event state resolves outside its namespace");
    }
    // Validate fail-closed and atomically repair permissions even when the
    // state was created by an older adapter or a permissive umask.
    await this.withManifestLock(async () => {});
    await this.prune(new Set());
    await this.assertConversationCapacity();
  }

  async resolve(
    conversationId: string,
    resetToken: string | undefined,
    expectedCwd: string,
    create: (
      persistedSessionFile: string | undefined,
      lifecycleGeneration: string,
    ) => Promise<{
      sessionFile: string;
      piSessionId: string;
      cwd: string;
    }>,
  ): Promise<ResolveConversationResult> {
    validateConversationId(conversationId);
    validateResetToken(resetToken);
    const canonicalCwd = resolve(expectedCwd);
    validatePathString(canonicalCwd, "conversation cwd", MAX_CWD_CHARACTERS);
    await this.ensureConversationCapacity(conversationId);
    const conversationLock = await this.acquireConversationLock(conversationId);
    let result: ResolveConversationResult;
    try {
      const { mapping: existing, resetTombstone } =
        await this.getConversationState(conversationId);
      const pendingResetTombstone =
        resetTombstone?.status === "pending" ? resetTombstone : undefined;
      if (
        pendingResetTombstone?.resetToken !== undefined &&
        resetToken !== undefined &&
        pendingResetTombstone.resetToken !== resetToken
      ) {
        throw new Error(
          "resetToken must match the latest committed Buzz reset token",
        );
      }
      const resetChanged =
        resetToken !== undefined && existing?.lastResetToken !== resetToken;
      const supersededPendingEvents =
        resetChanged || pendingResetTombstone !== undefined;
      const existingLeaseDisposition =
        existing === undefined ? "none" : this.leaseDisposition(existing);
      const existingLeaseActive = existingLeaseDisposition === "active";
      const existingLeaseMayStillWrite =
        existingLeaseActive || existingLeaseDisposition === "uncertain";
      if (existingLeaseActive && !resetChanged) {
        throw new Error("Conversation is active in another Pi runtime");
      }

      const cwdChanged =
        existing !== undefined && resolve(existing.cwd) !== canonicalCwd;
      let forceFresh =
        cwdChanged ||
        resetChanged ||
        pendingResetTombstone !== undefined ||
        existingLeaseDisposition === "uncertain";
      const lifecycleGeneration = supersededPendingEvents
        ? newLifecycleGeneration()
        : (existing?.lifecycleGeneration ?? newLifecycleGeneration());
      let replacedStale = false;
      let created: { sessionFile: string; piSessionId: string; cwd: string };
      try {
        created = await create(
          forceFresh ? undefined : existing?.sessionFile,
          lifecycleGeneration,
        );
      } catch (error) {
        if (!existing || forceFresh || !isRecoverableStaleSessionError(error)) {
          throw error;
        }
        this.logger.warn(
          "persisted Pi conversation was stale; recreating safely",
          {
            conversationId,
            error: errorMessage(error),
          },
        );
        forceFresh = true;
        replacedStale = true;
        created = await create(undefined, lifecycleGeneration);
      }

      if (resolve(created.cwd) !== canonicalCwd) {
        throw new Error("Pi session cwd did not match the requested workspace");
      }
      const sessionFile = await this.validateSessionFileOnDisk(
        created.sessionFile,
      );
      validatePiSessionId(created.piSessionId);
      await chmod(sessionFile, 0o600).catch((error: unknown) => {
        if (!isCode(error, "ENOENT")) throw error;
      });

      const now = new Date().toISOString();
      const lastResetToken =
        resetToken ??
        pendingResetTombstone?.resetToken ??
        existing?.lastResetToken;
      const relayHistoryCleared =
        existing?.relayHistoryCleared === true ||
        resetChanged ||
        pendingResetTombstone !== undefined;
      const lease = this.newLease();
      const mapping: ConversationMapping = {
        conversationId,
        cwd: canonicalCwd,
        sessionFile,
        piSessionId: created.piSessionId,
        lifecycleGeneration,
        createdAt: existing?.createdAt ?? now,
        lastUsedAt: now,
        ...(lastResetToken === undefined ? {} : { lastResetToken }),
        ...(relayHistoryCleared ? { relayHistoryCleared: true } : {}),
        lease,
      };
      await this.withManifestLock(async (manifest) => {
        await conversationLock.assertOwned();
        if (
          manifest.conversations[conversationId] === undefined &&
          Object.keys(manifest.conversations).length >=
            this.config.maxPersistedConversations
        ) {
          throw new Error(
            `Persisted Pi conversation capacity ${this.config.maxPersistedConversations} is full; no inactive mapping is safe to prune`,
          );
        }
        manifest.conversations[conversationId] = mapping;
        if (pendingResetTombstone !== undefined) {
          manifest.resetTombstones[conversationId] = {
            ...pendingResetTombstone,
            status: "retained",
            installedPiSessionId: mapping.piSessionId,
            consumedAt: now,
          };
        } else if (resetChanged) {
          manifest.resetTombstones[conversationId] = {
            conversationId,
            ...(existing?.piSessionId === undefined
              ? {}
              : { previousPiSessionId: existing.piSessionId }),
            ...(lastResetToken === undefined
              ? {}
              : { resetToken: lastResetToken }),
            createdAt: now,
            status: "retained",
            installedPiSessionId: mapping.piSessionId,
            consumedAt: now,
          };
        } else if (resetTombstone?.status === "retained") {
          manifest.resetTombstones[conversationId] = {
            ...resetTombstone,
            installedPiSessionId: mapping.piSessionId,
          };
        }
      });
      const replaced =
        (existing !== undefined && (forceFresh || replacedStale)) ||
        pendingResetTombstone !== undefined;
      const previousPiSessionId =
        existing?.piSessionId ?? pendingResetTombstone?.previousPiSessionId;
      const resumed = existing !== undefined && !forceFresh && !replacedStale;
      result = {
        mapping: structuredClone(mapping),
        lifecycleGeneration,
        resumed,
        ...(replaced && previousPiSessionId !== undefined
          ? { previousPiSessionId }
          : {}),
        // A changed, authenticated reset token may supersede another process's
        // active lease. Keep that old JSONL until normal retention pruning;
        // the other process can still be finishing an in-flight write.
        ...(replaced &&
        !existingLeaseMayStillWrite &&
        existing !== undefined &&
        existing.sessionFile !== sessionFile
          ? { retiredSessionFile: existing.sessionFile }
          : {}),
        skipRelayHistory: !resumed && relayHistoryCleared,
        refresh: () =>
          this.touch(
            conversationId,
            mapping.piSessionId,
            mapping.lastResetToken,
            lease.ownerId,
            mapping.lifecycleGeneration,
            mapping.sessionFile,
          ),
        forget: () =>
          this.forget(
            conversationId,
            mapping.piSessionId,
            mapping.lastResetToken,
            lease.ownerId,
            mapping.lifecycleGeneration,
            mapping.sessionFile,
          ),
        release: async () => {
          const stillReferenced = await this.release(
            conversationId,
            mapping.piSessionId,
            mapping.lastResetToken,
            lease.ownerId,
            mapping.lifecycleGeneration,
            mapping.sessionFile,
          );
          // A cross-adapter /new can supersede an active mapping. Its old file
          // becomes safe to remove only when the old handle is actually being
          // disposed and invokes this release closure.
          if (!stillReferenced)
            await this.safeDeleteSessionFile(mapping.sessionFile);
        },
      };
      if (supersededPendingEvents) {
        await this.clearPendingSessionEvents(
          conversationId,
          lifecycleGeneration,
        ).catch((error: unknown) => {
          // The epoch fence is already durable. Keep the fresh session usable;
          // any record that could not be cleaned is still classified by epoch
          // during replay rather than being mistaken for a non-reset recovery.
          this.logger.warn("deferred stale lifecycle-event cleanup failed", {
            conversationId,
            error: errorMessage(error),
          });
        });
      }
    } finally {
      await conversationLock.release();
    }

    // Capacity pruning is deliberately outside the conversation lock. Every
    // victim gets its own lock, avoiding manifest/conversation lock inversion.
    await this.prune(new Set([conversationId])).catch((error: unknown) => {
      this.logger.warn("deferred Pi conversation pruning failed", {
        error: errorMessage(error),
      });
    });
    return result;
  }

  async touch(
    conversationId: string,
    expectedPiSessionId: string,
    expectedResetToken: string | undefined,
    expectedLeaseOwnerId?: string,
    expectedLifecycleGeneration?: string,
    expectedSessionFile?: string,
  ): Promise<boolean> {
    validateConversationId(conversationId);
    return this.withManifestLock(async (manifest) => {
      const mapping = manifest.conversations[conversationId];
      if (
        !mapping ||
        mapping.piSessionId !== expectedPiSessionId ||
        mapping.lastResetToken !== expectedResetToken ||
        mapping.lease?.ownerId !== expectedLeaseOwnerId ||
        (expectedLifecycleGeneration !== undefined &&
          mapping.lifecycleGeneration !== expectedLifecycleGeneration) ||
        (expectedSessionFile !== undefined &&
          mapping.sessionFile !== expectedSessionFile)
      )
        return false;
      mapping.lastUsedAt = new Date().toISOString();
      mapping.lease = this.newLease(expectedLeaseOwnerId);
      return true;
    });
  }

  private async forget(
    conversationId: string,
    expectedPiSessionId: string,
    expectedResetToken: string | undefined,
    expectedLeaseOwnerId: string,
    expectedLifecycleGeneration: string,
    expectedSessionFile: string,
  ): Promise<string | undefined> {
    validateConversationId(conversationId);
    const conversationLock = await this.acquireConversationLock(conversationId);
    let sessionFile: string | undefined;
    try {
      await this.withManifestLock(async (manifest) => {
        await conversationLock.assertOwned();
        const mapping = manifest.conversations[conversationId];
        if (
          !mapping ||
          mapping.piSessionId !== expectedPiSessionId ||
          mapping.lastResetToken !== expectedResetToken ||
          mapping.lease?.ownerId !== expectedLeaseOwnerId ||
          mapping.lifecycleGeneration !== expectedLifecycleGeneration ||
          mapping.sessionFile !== expectedSessionFile
        )
          return;
        sessionFile = mapping.sessionFile;
        this.assertPendingResetCapacity(manifest, conversationId);
        manifest.resetTombstones[conversationId] = {
          conversationId,
          previousPiSessionId: mapping.piSessionId,
          createdAt: new Date().toISOString(),
          status: "pending",
        };
        delete manifest.conversations[conversationId];
      });
      if (sessionFile !== undefined) {
        await this.clearPendingSessionEvents(conversationId);
      }
      return sessionFile;
    } finally {
      await conversationLock.release();
    }
  }

  async deleteSessionFile(path: string): Promise<void> {
    await this.safeDeleteSessionFile(path);
  }

  async commitReset(
    conversationId: string,
    resetToken: string,
  ): Promise<CommitConversationResetResult> {
    validateConversationId(conversationId);
    validateResetToken(resetToken);
    const conversationLock = await this.acquireConversationLock(conversationId);
    try {
      const { result, shouldClearPendingEvents } = await this.withManifestLock(
        async (manifest) => {
          await conversationLock.assertOwned();
          const mapping = manifest.conversations[conversationId];
          const tombstone = manifest.resetTombstones[conversationId];
          if (
            mapping !== undefined &&
            mapping.lastResetToken === resetToken &&
            tombstone?.status !== "pending"
          ) {
            return {
              result: { alreadyCommitted: true, disposeLiveSession: false },
              shouldClearPendingEvents: false,
            };
          }
          if (
            tombstone?.status === "pending" &&
            tombstone.resetToken === resetToken
          ) {
            return {
              result: { alreadyCommitted: true, disposeLiveSession: true },
              shouldClearPendingEvents: true,
            };
          }

          const mappingLeaseMayStillWrite =
            mapping !== undefined && this.leaseMayStillWrite(mapping);
          const previousPiSessionId =
            mapping?.piSessionId ?? tombstone?.previousPiSessionId;
          this.assertPendingResetCapacity(manifest, conversationId);
          manifest.resetTombstones[conversationId] = {
            conversationId,
            ...(previousPiSessionId === undefined
              ? {}
              : { previousPiSessionId }),
            resetToken,
            createdAt: new Date().toISOString(),
            status: "pending",
          };
          delete manifest.conversations[conversationId];
          return {
            result: {
              alreadyCommitted: false,
              disposeLiveSession: true,
              ...(mapping !== undefined && !mappingLeaseMayStillWrite
                ? { retiredSessionFile: mapping.sessionFile }
                : {}),
            },
            shouldClearPendingEvents: true,
          };
        },
      );
      // The reset tombstone is committed first. If cleanup fails or the process
      // crashes here, replay generation checks still suppress old notices and
      // an idempotent reset retry completes the deletion before ACK.
      if (shouldClearPendingEvents) {
        await this.clearPendingSessionEvents(conversationId);
      }
      return result;
    } finally {
      await conversationLock.release();
    }
  }

  async release(
    conversationId: string,
    expectedPiSessionId: string,
    expectedResetToken: string | undefined,
    expectedLeaseOwnerId: string,
    expectedLifecycleGeneration: string,
    expectedSessionFile?: string,
  ): Promise<boolean> {
    validateConversationId(conversationId);
    return this.withManifestLock(async (manifest) => {
      const mapping = manifest.conversations[conversationId];
      if (
        mapping?.piSessionId === expectedPiSessionId &&
        mapping.lastResetToken === expectedResetToken &&
        mapping.lease?.ownerId === expectedLeaseOwnerId &&
        mapping.lifecycleGeneration === expectedLifecycleGeneration &&
        (expectedSessionFile === undefined ||
          mapping.sessionFile === expectedSessionFile)
      ) {
        delete mapping.lease;
      }
      return (
        expectedSessionFile === undefined ||
        mapping?.sessionFile === expectedSessionFile
      );
    });
  }

  async prune(
    activeConversationIds: Set<string>,
    reserveSlots = 0,
  ): Promise<number> {
    const snapshot = await this.withManifestLock(
      async (manifest) => structuredClone(manifest),
      false,
    );
    const candidates = this.selectPruneCandidates(
      snapshot,
      activeConversationIds,
    );
    const requiredRemovalCount = Math.max(
      0,
      Object.keys(snapshot.conversations).length -
        Math.max(0, this.config.maxPersistedConversations - reserveSlots),
    );
    let pruned = 0;
    for (const candidate of candidates) {
      if (candidate.reason === "capacity" && pruned >= requiredRemovalCount) {
        break;
      }
      const conversationLock = await this.acquireConversationLock(
        candidate.mapping.conversationId,
      );
      let removedFile: string | undefined;
      try {
        // Pending events pin their mapping. Hold this lock across the final
        // manifest revalidation/removal: enqueue uses the same pending-events
        // -> manifest order, so either its durable write wins and pruning
        // skips, or pruning removes the mapping and enqueue rejects the epoch.
        const pendingEventsLock = await this.acquireLock("pending-events");
        try {
          if (
            await this.hasPendingSessionEventsLocked(
              candidate.mapping.conversationId,
            )
          ) {
            continue;
          }
          await this.withManifestLock(async (manifest) => {
            await conversationLock.assertOwned();
            await pendingEventsLock.assertOwned();
            const current =
              manifest.conversations[candidate.mapping.conversationId];
            if (
              !current ||
              activeConversationIds.has(current.conversationId) ||
              this.leaseMayStillWrite(current) ||
              !sameMappingVersion(current, candidate.mapping)
            )
              return;
            if (
              candidate.reason === "ttl" &&
              Date.now() - Date.parse(current.lastUsedAt) <
                this.config.persistedConversationTtlMs
            )
              return;
            if (current.relayHistoryCleared === true) {
              try {
                this.assertPendingResetCapacity(
                  manifest,
                  current.conversationId,
                );
              } catch (error) {
                this.logger.warn(
                  "retained reset barrier prevented Pi mapping pruning",
                  {
                    conversationId: current.conversationId,
                    error: errorMessage(error),
                  },
                );
                return;
              }
              const retained = manifest.resetTombstones[current.conversationId];
              manifest.resetTombstones[current.conversationId] = {
                conversationId: current.conversationId,
                previousPiSessionId: current.piSessionId,
                ...(current.lastResetToken === undefined
                  ? {}
                  : { resetToken: current.lastResetToken }),
                createdAt: retained?.createdAt ?? new Date().toISOString(),
                status: "pending",
              };
            }
            removedFile = current.sessionFile;
            delete manifest.conversations[current.conversationId];
            pruned++;
          });
        } finally {
          await pendingEventsLock.release();
        }
      } finally {
        await conversationLock.release();
      }
      if (removedFile) await this.safeDeleteSessionFile(removedFile);
    }
    if (pruned > 0) {
      this.logger.info("pruned persisted Pi conversations", {
        count: pruned,
        namespace: this.namespace,
      });
    }
    return pruned;
  }

  async get(conversationId: string): Promise<ConversationMapping | undefined> {
    validateConversationId(conversationId);
    return this.withManifestLock(async (manifest) => {
      const mapping = manifest.conversations[conversationId];
      return mapping ? structuredClone(mapping) : undefined;
    }, false);
  }

  async enqueueSessionEvent(
    conversationId: string,
    eventId: string,
    event: BuzzSessionEvent,
    expectedLifecycleGeneration: string,
  ): Promise<boolean> {
    validateConversationId(conversationId);
    validateEventId(eventId);
    validateStoredSessionEvent(event);
    validateLifecycleGeneration(expectedLifecycleGeneration);
    const record: PendingBuzzSessionEvent = {
      conversationId,
      eventId,
      lifecycleGeneration: expectedLifecycleGeneration,
      event: structuredClone(event),
      createdAt: new Date().toISOString(),
    };
    const serialized = `${JSON.stringify({ version: 1, ...record })}\n`;
    if (
      Buffer.byteLength(serialized, "utf8") > MAX_PENDING_SESSION_EVENT_BYTES
    ) {
      throw new Error(
        `Buzz session lifecycle event exceeds ${MAX_PENDING_SESSION_EVENT_BYTES} bytes`,
      );
    }
    const lock = await this.acquireLock("pending-events");
    try {
      const currentMapping = await this.get(conversationId);
      if (currentMapping?.lifecycleGeneration !== expectedLifecycleGeneration) {
        // A reset may commit after the runtime emitted an event but before its
        // asynchronous durable write. Epoch fencing rejects that stale notice,
        // while a non-reset Pi replacement retains the same epoch and may still
        // finish a valid in-flight write.
        return false;
      }
      const directory = this.conversationEventDirectory(conversationId);
      const target = join(directory, `${eventId}.json`);
      const existing = await this.readPendingSessionEventFile(
        target,
        currentMapping.lifecycleGeneration,
      ).catch((error: unknown) => {
        if (isCode(error, "ENOENT")) return undefined;
        throw error;
      });
      if (existing) {
        if (
          existing.conversationId === conversationId &&
          existing.eventId === eventId &&
          existing.lifecycleGeneration === expectedLifecycleGeneration &&
          JSON.stringify(existing.event) === JSON.stringify(event)
        ) {
          return true;
        }
        throw new Error("Buzz session lifecycle event id collision");
      }
      const count = await this.countPendingSessionEvents();
      if (count >= this.config.maxPendingSessionEvents) {
        throw new Error(
          `Pending Buzz session lifecycle event capacity ${this.config.maxPendingSessionEvents} is full; refusing to drop an unacknowledged notice`,
        );
      }
      await this.ensureConversationEventDirectory(conversationId);
      const temporary = join(
        directory,
        `.event.${process.pid}.${randomUUID()}.tmp`,
      );
      let handle: Awaited<ReturnType<typeof open>> | undefined;
      try {
        handle = await open(temporary, "wx", 0o600);
        await handle.writeFile(serialized, { encoding: "utf8" });
        await handle.chmod(0o600);
        await handle.sync();
        await handle.close();
        handle = undefined;
        await lock.assertOwned();
        await rename(temporary, target);
        await syncDirectoryEntry(directory);
        return true;
      } finally {
        await handle?.close().catch(() => {});
        await rm(temporary, { force: true }).catch(() => {});
      }
    } finally {
      await lock.release();
    }
  }

  async listPendingSessionEvents(
    conversationId: string,
  ): Promise<PendingBuzzSessionEvent[]> {
    validateConversationId(conversationId);
    const lock = await this.acquireLock("pending-events");
    try {
      // Lock order is pending-events -> manifest everywhere these overlap.
      // Legacy records had no explicit epoch; conservatively attach them to
      // the current mapping so an upgrade cannot silently lose an ACK-pending
      // notice whose Pi ID changed during a non-reset recovery.
      const currentLifecycleGeneration = (await this.get(conversationId))
        ?.lifecycleGeneration;
      const directory = this.conversationEventDirectory(conversationId);
      let entries: Awaited<ReturnType<typeof opendir>>;
      try {
        entries = await opendir(directory);
      } catch (error) {
        if (isCode(error, "ENOENT")) return [];
        throw error;
      }
      const records: PendingBuzzSessionEvent[] = [];
      for await (const entry of entries) {
        if (isEventTemporaryFile(entry.name) && entry.isFile()) {
          await unlink(join(directory, entry.name)).catch(() => {});
          continue;
        }
        if (!entry.isFile() || !entry.name.endsWith(".json")) {
          throw new Error("invalid pending Buzz session lifecycle entry");
        }
        const eventId = entry.name.slice(0, -".json".length);
        validateEventId(eventId);
        const record = await this.readPendingSessionEventFile(
          join(directory, entry.name),
          currentLifecycleGeneration,
        );
        if (
          record.conversationId !== conversationId ||
          record.eventId !== eventId
        ) {
          throw new Error("pending Buzz session lifecycle identity mismatch");
        }
        records.push(record);
        if (records.length > this.config.maxPendingSessionEvents) {
          throw new Error(
            "pending Buzz session lifecycle state exceeds capacity",
          );
        }
      }
      return records.sort(
        (left, right) =>
          Date.parse(left.createdAt) - Date.parse(right.createdAt) ||
          left.eventId.localeCompare(right.eventId),
      );
    } finally {
      await lock.release();
    }
  }

  async acknowledgeSessionEvent(
    conversationId: string,
    eventId: string,
  ): Promise<void> {
    validateConversationId(conversationId);
    validateEventId(eventId);
    const lock = await this.acquireLock("pending-events");
    try {
      const directory = this.conversationEventDirectory(conversationId);
      const target = join(directory, `${eventId}.json`);
      let record: PendingBuzzSessionEvent;
      try {
        record = await this.readPendingSessionEventFile(target);
      } catch (error) {
        if (isCode(error, "ENOENT")) return;
        throw error;
      }
      if (
        record.conversationId !== conversationId ||
        record.eventId !== eventId
      ) {
        throw new Error("pending Buzz session lifecycle identity mismatch");
      }
      await lock.assertOwned();
      await unlink(target);
      await syncDirectoryEntry(directory);
      await rmdir(directory).catch((error: unknown) => {
        if (!isCode(error, "ENOTEMPTY") && !isCode(error, "ENOENT")) {
          throw error;
        }
      });
      await syncDirectoryEntry(this.pendingEventsDirectory);
    } finally {
      await lock.release();
    }
  }

  async clearPendingSessionEvents(
    conversationId: string,
    preserveLifecycleGeneration?: string,
  ): Promise<void> {
    validateConversationId(conversationId);
    if (preserveLifecycleGeneration !== undefined) {
      validateLifecycleGeneration(preserveLifecycleGeneration);
    }
    const lock = await this.acquireLock("pending-events");
    try {
      const directory = this.conversationEventDirectory(conversationId);
      let metadata: Awaited<ReturnType<typeof lstat>>;
      try {
        metadata = await lstat(directory);
      } catch (error) {
        if (isCode(error, "ENOENT")) return;
        throw error;
      }
      if (metadata.isSymbolicLink() || !metadata.isDirectory()) {
        throw new Error("invalid pending Buzz session lifecycle directory");
      }
      const entries = await opendir(directory);
      let removedAny = false;
      for await (const entry of entries) {
        if (
          !entry.isFile() ||
          (!isEventTemporaryFile(entry.name) && !entry.name.endsWith(".json"))
        ) {
          throw new Error("invalid pending Buzz session lifecycle entry");
        }
        if (entry.name.endsWith(".json")) {
          validateEventId(entry.name.slice(0, -".json".length));
          if (preserveLifecycleGeneration !== undefined) {
            const record = await this.readPendingSessionEventFile(
              join(directory, entry.name),
            );
            if (record.lifecycleGeneration === preserveLifecycleGeneration) {
              continue;
            }
          }
        }
        await unlink(join(directory, entry.name));
        removedAny = true;
      }
      if (removedAny) await syncDirectoryEntry(directory);
      let removedDirectory = false;
      try {
        await rmdir(directory);
        removedDirectory = true;
      } catch (error) {
        if (!isCode(error, "ENOTEMPTY") && !isCode(error, "ENOENT")) {
          throw error;
        }
      }
      if (removedAny || removedDirectory) {
        await syncDirectoryEntry(this.pendingEventsDirectory);
      }
    } finally {
      await lock.release();
    }
  }

  private conversationEventDirectory(conversationId: string): string {
    return join(this.pendingEventsDirectory, hash(conversationId));
  }

  private async ensureConversationEventDirectory(
    conversationId: string,
  ): Promise<string> {
    const directory = this.conversationEventDirectory(conversationId);
    await mkdir(directory, { recursive: true, mode: 0o700 });
    await chmod(directory, 0o700);
    const metadata = await lstat(directory);
    if (metadata.isSymbolicLink() || !metadata.isDirectory()) {
      throw new Error("invalid pending Buzz session lifecycle directory");
    }
    const physical = await realpath(directory);
    const physicalRoot = await realpath(this.pendingEventsDirectory);
    if (!pathIsStrictlyWithin(physical, physicalRoot)) {
      throw new Error("pending Buzz session lifecycle directory escaped state");
    }
    await syncDirectoryEntry(this.pendingEventsDirectory);
    return directory;
  }

  private async countPendingSessionEvents(): Promise<number> {
    const root = await opendir(this.pendingEventsDirectory);
    let count = 0;
    let scanned = 0;
    for await (const directoryEntry of root) {
      scanned++;
      if (scanned > this.config.maxPendingSessionEvents * 2 + 32) {
        throw new Error("pending Buzz session lifecycle state is unbounded");
      }
      if (
        !directoryEntry.isDirectory() ||
        !EVENT_DIRECTORY_PATTERN.test(directoryEntry.name)
      ) {
        throw new Error("invalid pending Buzz session lifecycle directory");
      }
      const directory = await opendir(
        join(this.pendingEventsDirectory, directoryEntry.name),
      );
      for await (const entry of directory) {
        scanned++;
        if (scanned > this.config.maxPendingSessionEvents * 3 + 64) {
          throw new Error("pending Buzz session lifecycle state is unbounded");
        }
        if (isEventTemporaryFile(entry.name) && entry.isFile()) {
          await unlink(
            join(this.pendingEventsDirectory, directoryEntry.name, entry.name),
          ).catch(() => {});
          continue;
        }
        if (!entry.isFile() || !entry.name.endsWith(".json")) {
          throw new Error("invalid pending Buzz session lifecycle entry");
        }
        validateEventId(entry.name.slice(0, -".json".length));
        count++;
        if (count >= this.config.maxPendingSessionEvents) return count;
      }
    }
    return count;
  }

  /** Requires the caller to own the global pending-events lock. */
  private async hasPendingSessionEventsLocked(
    conversationId: string,
  ): Promise<boolean> {
    const directory = this.conversationEventDirectory(conversationId);
    let entries: Awaited<ReturnType<typeof opendir>>;
    try {
      entries = await opendir(directory);
    } catch (error) {
      if (isCode(error, "ENOENT")) return false;
      throw error;
    }
    let hasPending = false;
    let removedTemporary = false;
    let scanned = 0;
    for await (const entry of entries) {
      scanned++;
      if (scanned > this.config.maxPendingSessionEvents + 32) {
        throw new Error("pending Buzz session lifecycle state is unbounded");
      }
      if (isEventTemporaryFile(entry.name) && entry.isFile()) {
        await unlink(join(directory, entry.name));
        removedTemporary = true;
        continue;
      }
      if (!entry.isFile() || !entry.name.endsWith(".json")) {
        throw new Error("invalid pending Buzz session lifecycle entry");
      }
      const eventId = entry.name.slice(0, -".json".length);
      validateEventId(eventId);
      const record = await this.readPendingSessionEventFile(
        join(directory, entry.name),
      );
      if (
        record.conversationId !== conversationId ||
        record.eventId !== eventId
      ) {
        throw new Error("pending Buzz session lifecycle identity mismatch");
      }
      hasPending = true;
    }
    if (removedTemporary) await syncDirectoryEntry(directory);
    return hasPending;
  }

  private async readPendingSessionEventFile(
    path: string,
    legacyLifecycleGeneration?: string,
  ): Promise<PendingBuzzSessionEvent> {
    const metadata = await lstat(path);
    if (
      metadata.isSymbolicLink() ||
      !metadata.isFile() ||
      metadata.size > MAX_PENDING_SESSION_EVENT_BYTES
    ) {
      throw new Error("invalid pending Buzz session lifecycle file");
    }
    await chmod(path, 0o600);
    const raw: unknown = JSON.parse(await readFile(path, "utf8"));
    if (!isRecord(raw) || raw.version !== 1) {
      throw new Error("invalid pending Buzz session lifecycle record");
    }
    const conversationId = requiredBoundedString(
      raw.conversationId,
      "pendingEvent.conversationId",
      512,
    );
    validateConversationId(conversationId);
    const eventId = requiredBoundedString(
      raw.eventId,
      "pendingEvent.eventId",
      64,
    );
    validateEventId(eventId);
    validateStoredSessionEvent(raw.event);
    const lifecycleGeneration =
      raw.lifecycleGeneration === undefined
        ? (legacyLifecycleGeneration ??
          deriveLegacyLifecycleGeneration(raw.event.piSessionId))
        : requiredBoundedString(
            raw.lifecycleGeneration,
            "pendingEvent.lifecycleGeneration",
            64,
          );
    validateLifecycleGeneration(lifecycleGeneration);
    return {
      conversationId,
      eventId,
      lifecycleGeneration,
      event: raw.event,
      createdAt: validIsoDate(raw.createdAt, "pendingEvent.createdAt"),
    };
  }

  private async getConversationState(conversationId: string): Promise<{
    mapping: ConversationMapping | undefined;
    resetTombstone: ResetTombstone | undefined;
  }> {
    return this.withManifestLock(async (manifest) => {
      const mapping = manifest.conversations[conversationId];
      const resetTombstone = manifest.resetTombstones[conversationId];
      return {
        mapping: mapping ? structuredClone(mapping) : undefined,
        resetTombstone: resetTombstone
          ? structuredClone(resetTombstone)
          : undefined,
      };
    }, false);
  }

  private selectPruneCandidates(
    manifest: ConversationManifest,
    activeConversationIds: Set<string>,
  ): PruneCandidate[] {
    const now = Date.now();
    const entries = Object.values(manifest.conversations).sort(
      (left, right) =>
        Date.parse(left.lastUsedAt) - Date.parse(right.lastUsedAt),
    );
    const expired = entries.filter(
      (mapping) =>
        !activeConversationIds.has(mapping.conversationId) &&
        !this.leaseMayStillWrite(mapping) &&
        now - Date.parse(mapping.lastUsedAt) >=
          this.config.persistedConversationTtlMs,
    );
    const expiredIds = new Set(
      expired.map((mapping) => mapping.conversationId),
    );
    const excess = entries.filter(
      (mapping) =>
        !expiredIds.has(mapping.conversationId) &&
        !activeConversationIds.has(mapping.conversationId) &&
        !this.leaseMayStillWrite(mapping),
    );
    return [
      ...expired.map((mapping) => ({ mapping, reason: "ttl" as const })),
      ...excess.map((mapping) => ({ mapping, reason: "capacity" as const })),
    ];
  }

  private async ensureConversationCapacity(
    conversationId: string,
  ): Promise<void> {
    const existing = await this.get(conversationId);
    if (existing) return;
    await this.prune(new Set([conversationId]), 1);
    await this.assertConversationCapacity(conversationId);
  }

  private async assertConversationCapacity(
    insertingConversationId?: string,
  ): Promise<void> {
    await this.withManifestLock(async (manifest) => {
      if (
        insertingConversationId !== undefined &&
        manifest.conversations[insertingConversationId] !== undefined
      ) {
        return;
      }
      const count = Object.keys(manifest.conversations).length;
      const overCapacity =
        insertingConversationId === undefined
          ? count > this.config.maxPersistedConversations
          : count >= this.config.maxPersistedConversations;
      if (overCapacity) {
        throw new Error(
          `Persisted Pi conversation capacity ${this.config.maxPersistedConversations} is full; no inactive mapping is safe to prune`,
        );
      }
    }, false);
  }

  private assertPendingResetCapacity(
    manifest: ConversationManifest,
    conversationId: string,
  ): void {
    if (manifest.resetTombstones[conversationId]?.status === "pending") return;
    const pendingCount = Object.values(manifest.resetTombstones).filter(
      (tombstone) => tombstone.status === "pending",
    ).length;
    if (pendingCount >= this.config.maxPendingResetTombstones) {
      throw new Error(
        `Pending reset tombstone capacity ${this.config.maxPendingResetTombstones} is full; a cleared thread must be opened to install its fresh Pi mapping before committing another reset`,
      );
    }
  }

  private pruneRetainedResetTombstones(manifest: ConversationManifest): number {
    const now = Date.now();
    const safelyRetained: Array<
      [string, Extract<ResetTombstone, { status: "retained" }>]
    > = [];
    for (const [conversationId, tombstone] of Object.entries(
      manifest.resetTombstones,
    )) {
      if (tombstone.status !== "retained") continue;
      const mapping = manifest.conversations[conversationId];
      if (
        mapping?.relayHistoryCleared !== true ||
        mapping.piSessionId !== tombstone.installedPiSessionId ||
        (tombstone.resetToken !== undefined &&
          mapping.lastResetToken !== tombstone.resetToken)
      ) {
        // Never discard an unanchored barrier. Re-activate it so token-less
        // session/new is forced fresh and skips relay history.
        manifest.resetTombstones[conversationId] = {
          conversationId,
          ...(tombstone.previousPiSessionId === undefined
            ? {}
            : { previousPiSessionId: tombstone.previousPiSessionId }),
          ...(tombstone.resetToken === undefined
            ? {}
            : { resetToken: tombstone.resetToken }),
          createdAt: tombstone.createdAt,
          status: "pending",
        };
        continue;
      }
      safelyRetained.push([conversationId, tombstone]);
    }

    const expired = safelyRetained.filter(
      ([, tombstone]) =>
        now - Date.parse(tombstone.consumedAt) >=
        this.config.resetTombstoneTtlMs,
    );
    const expiredIds = new Set(
      expired.map(([conversationId]) => conversationId),
    );
    const remaining = safelyRetained
      .filter(([conversationId]) => !expiredIds.has(conversationId))
      .sort(
        (left, right) =>
          Date.parse(left[1].consumedAt) - Date.parse(right[1].consumedAt),
      );
    const excess = remaining.slice(
      0,
      Math.max(0, remaining.length - this.config.maxRetainedResetTombstones),
    );
    for (const [conversationId] of [...expired, ...excess]) {
      delete manifest.resetTombstones[conversationId];
    }
    return expired.length + excess.length;
  }

  private leaseMayStillWrite(mapping: ConversationMapping): boolean {
    const disposition = this.leaseDisposition(mapping);
    return disposition === "active" || disposition === "uncertain";
  }

  private leaseDisposition(
    mapping: ConversationMapping,
  ): "none" | "active" | "proven-dead" | "uncertain" {
    const lease = mapping.lease;
    if (!lease) return "none";
    const unexpired = Date.parse(lease.expiresAt) > Date.now();

    // PID liveness is meaningful only on the host that wrote the lease. For a
    // foreign or legacy lease, an expired TTL permits takeover but cannot
    // prove the predecessor is dead. That takeover must use a fresh JSONL.
    if (
      lease.hostId !== this.leaseIdentity.hostId ||
      !this.leaseIdentity.pidProbeSafe
    ) {
      return unexpired ? "active" : "uncertain";
    }

    // A reboot makes every process from the prior boot definitively stale,
    // even if its PID has since been reused by an unrelated process.
    if (
      lease.bootId !== undefined &&
      this.leaseIdentity.bootId !== undefined &&
      lease.bootId !== this.leaseIdentity.bootId
    ) {
      return "proven-dead";
    }

    // A confirmed-dead process on this host can safely resume its JSONL. A
    // live PID with an expired lease may merely be suspended, so it is an
    // uncertain takeover and must be isolated onto a fresh file.
    if (!isProcessAlive(lease.pid)) return "proven-dead";
    if (!unexpired) return "uncertain";

    return "active";
  }

  private newLease(
    ownerId: string = randomUUID(),
  ): NonNullable<ConversationMapping["lease"]> {
    return {
      ownerId,
      pid: process.pid,
      hostId: this.leaseIdentity.hostId,
      ...(this.leaseIdentity.bootId === undefined
        ? {}
        : { bootId: this.leaseIdentity.bootId }),
      expiresAt: new Date(
        Date.now() + this.config.conversationLeaseMs,
      ).toISOString(),
    };
  }

  private async withManifestLock<T>(
    operation: (manifest: ConversationManifest) => Promise<T>,
    write = true,
  ): Promise<T> {
    const lock = await this.acquireLock("manifest");
    try {
      const manifest = await this.readManifest();
      const result = await operation(manifest);
      if (write) {
        this.pruneRetainedResetTombstones(manifest);
        await lock.assertOwned();
        await this.writeManifest(manifest, lock.assertOwned);
      }
      return result;
    } finally {
      await lock.release();
    }
  }

  private async readManifest(): Promise<ConversationManifest> {
    try {
      const metadata = await stat(this.manifestPath);
      if (!metadata.isFile() || metadata.size > MAX_MANIFEST_BYTES) {
        throw new Error("invalid conversation manifest file");
      }
      await chmod(this.manifestPath, 0o600);
      const raw = await readFile(this.manifestPath, "utf8");
      return await this.validateManifest(JSON.parse(raw));
    } catch (error) {
      if (isCode(error, "ENOENT")) {
        try {
          const marker = await lstat(this.initializedMarkerPath);
          if (!marker.isFile() || marker.isSymbolicLink()) {
            throw new Error("invalid Pi state initialization marker");
          }
        } catch (markerError) {
          if (isCode(markerError, "ENOENT")) return emptyManifest();
          throw markerError;
        }
        throw new Error(
          "Pi conversation manifest is missing after durable initialization",
        );
      }
      this.logger.error("refused corrupt conversation manifest", {
        error: errorMessage(error),
      });
      throw new Error(
        `Pi conversation manifest is unreadable; refusing to lose durable reset boundaries: ${errorMessage(error)}`,
        { cause: error },
      );
    }
  }

  private async validateManifest(
    value: unknown,
  ): Promise<ConversationManifest> {
    if (
      !isRecord(value) ||
      value.version !== 1 ||
      !isRecord(value.conversations)
    ) {
      throw new Error("invalid conversation manifest schema");
    }
    const resetTombstonesValue = value.resetTombstones ?? {};
    if (!isRecord(resetTombstonesValue)) {
      throw new Error("invalid reset tombstone manifest schema");
    }
    const entries = Object.entries(value.conversations);
    const resetEntries = Object.entries(resetTombstonesValue);
    if (entries.length + resetEntries.length > MAX_MANIFEST_ENTRIES) {
      throw new Error("conversation manifest contains too many entries");
    }
    const conversations: Record<string, ConversationMapping> = {};
    for (const [key, raw] of entries) {
      validateConversationId(key);
      if (!isRecord(raw) || raw.conversationId !== key) {
        throw new Error("invalid conversation mapping identity");
      }
      const cwd = requiredBoundedString(raw.cwd, "cwd", MAX_CWD_CHARACTERS);
      if (!isAbsolute(cwd)) throw new Error("invalid conversation cwd");
      const sessionFile = await this.validateSessionFileOnDisk(
        requiredBoundedString(
          raw.sessionFile,
          "sessionFile",
          MAX_SESSION_PATH_CHARACTERS,
        ),
      );
      const piSessionId = requiredBoundedString(
        raw.piSessionId,
        "piSessionId",
        256,
      );
      validatePiSessionId(piSessionId);
      const lifecycleGeneration =
        raw.lifecycleGeneration === undefined
          ? deriveLegacyLifecycleGeneration(piSessionId)
          : requiredBoundedString(
              raw.lifecycleGeneration,
              "lifecycleGeneration",
              64,
            );
      validateLifecycleGeneration(lifecycleGeneration);
      const createdAt = validIsoDate(raw.createdAt, "createdAt");
      const lastUsedAt = validIsoDate(raw.lastUsedAt, "lastUsedAt");
      const lastResetToken = optionalBoundedString(
        raw.lastResetToken,
        "lastResetToken",
        512,
      );
      const relayHistoryCleared =
        raw.relayHistoryCleared === undefined
          ? lastResetToken !== undefined
          : requiredBoolean(raw.relayHistoryCleared, "relayHistoryCleared");
      const lease =
        raw.lease === undefined ? undefined : validateLease(raw.lease);
      conversations[key] = {
        conversationId: key,
        cwd: resolve(cwd),
        sessionFile,
        piSessionId,
        lifecycleGeneration,
        createdAt,
        lastUsedAt,
        ...(lastResetToken === undefined ? {} : { lastResetToken }),
        ...(relayHistoryCleared ? { relayHistoryCleared: true } : {}),
        ...(lease === undefined ? {} : { lease }),
      };
    }
    const resetTombstones: Record<string, ResetTombstone> = {};
    for (const [key, raw] of resetEntries) {
      validateConversationId(key);
      if (!isRecord(raw) || raw.conversationId !== key) {
        throw new Error("invalid reset tombstone identity");
      }
      const previousPiSessionId = optionalBoundedString(
        raw.previousPiSessionId,
        "resetTombstone.previousPiSessionId",
        256,
      );
      if (previousPiSessionId !== undefined)
        validatePiSessionId(previousPiSessionId);
      const resetToken = optionalBoundedString(
        raw.resetToken,
        "resetTombstone.resetToken",
        512,
      );
      const base: ResetTombstoneBase = {
        conversationId: key,
        ...(previousPiSessionId === undefined ? {} : { previousPiSessionId }),
        ...(resetToken === undefined ? {} : { resetToken }),
        createdAt: validIsoDate(raw.createdAt, "resetTombstone.createdAt"),
      };
      const status = raw.status ?? "pending";
      if (status === "pending") {
        if (conversations[key] !== undefined) {
          throw new Error("pending reset tombstone overlaps a mapping");
        }
        resetTombstones[key] = { ...base, status };
        continue;
      }
      if (status !== "retained") {
        throw new Error("invalid reset tombstone status");
      }
      const installedPiSessionId = requiredBoundedString(
        raw.installedPiSessionId,
        "resetTombstone.installedPiSessionId",
        256,
      );
      validatePiSessionId(installedPiSessionId);
      const consumedAt = validIsoDate(
        raw.consumedAt,
        "resetTombstone.consumedAt",
      );
      const mapping = conversations[key];
      if (
        mapping?.relayHistoryCleared !== true ||
        mapping.piSessionId !== installedPiSessionId ||
        (resetToken !== undefined && mapping.lastResetToken !== resetToken)
      ) {
        throw new Error("retained reset tombstone is not safely anchored");
      }
      resetTombstones[key] = {
        ...base,
        status,
        installedPiSessionId,
        consumedAt,
      };
    }
    return { version: 1, conversations, resetTombstones };
  }

  private async writeManifest(
    manifest: ConversationManifest,
    assertLockOwned: () => Promise<void>,
  ): Promise<void> {
    const serialized = `${JSON.stringify(manifest, null, 2)}\n`;
    const serializedBytes = Buffer.byteLength(serialized, "utf8");
    if (serializedBytes > MAX_MANIFEST_BYTES) {
      throw new Error(
        `conversation manifest serialization exceeds ${MAX_MANIFEST_BYTES} bytes`,
      );
    }
    await mkdir(this.directory, { recursive: true, mode: 0o700 });
    await chmod(this.directory, 0o700);
    const temporary = join(
      this.directory,
      `.${basename(this.manifestPath)}.${process.pid}.${randomUUID()}.tmp`,
    );
    let temporaryHandle: Awaited<ReturnType<typeof open>> | undefined;
    try {
      temporaryHandle = await open(temporary, "wx", 0o600);
      await temporaryHandle.writeFile(serialized, { encoding: "utf8" });
      await temporaryHandle.chmod(0o600);
      await temporaryHandle.sync();
      await temporaryHandle.close();
      temporaryHandle = undefined;
      // A long suspension can let another process retire this lock. Recheck
      // the acquisition token at the last possible point before publishing.
      await assertLockOwned();
      await rename(temporary, this.manifestPath);
      await syncDirectoryEntry(this.directory);
      await this.ensureInitializedMarker();
    } finally {
      await temporaryHandle?.close().catch(() => {});
      await rm(temporary, { force: true }).catch(() => {});
    }
  }

  private async ensureInitializedMarker(): Promise<void> {
    try {
      const marker = await lstat(this.initializedMarkerPath);
      if (marker.isFile() && !marker.isSymbolicLink()) {
        await chmod(this.initializedMarkerPath, 0o600);
        return;
      }
      throw new Error("invalid Pi state initialization marker");
    } catch (error) {
      if (!isCode(error, "ENOENT")) throw error;
    }

    const temporary = join(
      this.directory,
      `.${INITIALIZED_MARKER}.${process.pid}.${randomUUID()}.tmp`,
    );
    let handle: Awaited<ReturnType<typeof open>> | undefined;
    try {
      handle = await open(temporary, "wx", 0o600);
      await handle.writeFile("buzz-pi-state-v1\n", { encoding: "utf8" });
      await handle.chmod(0o600);
      await handle.sync();
      await handle.close();
      handle = undefined;
      try {
        await rename(temporary, this.initializedMarkerPath);
      } catch (error) {
        if (!isCode(error, "EEXIST")) throw error;
      }
      await syncDirectoryEntry(this.directory);
    } finally {
      await handle?.close().catch(() => {});
      await rm(temporary, { force: true }).catch(() => {});
    }
  }

  private validateSessionFile(path: string): string {
    validatePathString(path, "Pi session path", MAX_SESSION_PATH_CHARACTERS);
    const canonical = resolve(path);
    if (!isAbsolute(path) || !canonical.endsWith(".jsonl")) {
      throw new Error("invalid Pi session path");
    }
    if (
      !this.allowedSessionRoots.some((root) => pathIsWithin(canonical, root))
    ) {
      throw new Error(
        "Pi session path is outside an allowed session directory",
      );
    }
    return canonical;
  }

  private async validateSessionFileOnDisk(path: string): Promise<string> {
    const canonical = this.validateSessionFile(path);
    try {
      const physical = await realpath(canonical);
      const physicalRoots = await Promise.all(
        this.allowedSessionRoots.map(async (root) =>
          realpath(root).catch(() => root),
        ),
      );
      if (!physicalRoots.some((root) => pathIsWithin(physical, root))) {
        throw new Error(
          "Pi session symlink resolves outside an allowed session directory",
        );
      }
    } catch (error) {
      // A missing mapping is allowed through so SessionManager.open can fail
      // and the stale-session recovery path can atomically replace it.
      if (!isCode(error, "ENOENT")) throw error;
    }
    return canonical;
  }

  private async safeDeleteSessionFile(path: string): Promise<void> {
    let canonical: string;
    try {
      canonical = await this.validateSessionFileOnDisk(path);
    } catch (error) {
      this.logger.warn("refused to delete an invalid Pi session path", {
        error: errorMessage(error),
      });
      return;
    }
    try {
      await unlink(canonical);
    } catch (error) {
      if (!isCode(error, "ENOENT")) {
        this.logger.warn("failed to prune Pi session file", {
          error: errorMessage(error),
        });
      }
    }
  }

  private acquireConversationLock(
    conversationId: string,
  ): Promise<AcquiredStateLock> {
    return this.acquireLock(`conversation-${hash(conversationId)}`);
  }

  private async acquireLock(name: string): Promise<AcquiredStateLock> {
    await mkdir(this.locksDirectory, { recursive: true, mode: 0o700 });
    await chmod(this.locksDirectory, 0o700);
    const lockPath = join(this.locksDirectory, `${name}.lock`);
    const deadline = Date.now() + 30_000;
    while (true) {
      try {
        const acquisitionToken = randomUUID();
        await mkdir(lockPath, { mode: 0o700 });
        const ownerFile = join(lockPath, "owner");
        const owner: LockOwnerRecord = {
          version: 1,
          pid: process.pid,
          token: acquisitionToken,
          hostId: this.leaseIdentity.hostId,
          ...(this.leaseIdentity.bootId === undefined
            ? {}
            : { bootId: this.leaseIdentity.bootId }),
          createdAt: new Date().toISOString(),
        };
        await writeFile(ownerFile, `${JSON.stringify(owner)}\n`, {
          mode: 0o600,
        });
        await chmod(ownerFile, 0o600);
        const heartbeatFile = join(lockPath, `heartbeat-${acquisitionToken}`);
        await writeFile(heartbeatFile, "\n", { mode: 0o600 });
        const ownedGeneration = await captureStateLockGeneration(lockPath);
        if (
          !ownedGeneration ||
          parseLockOwner(ownedGeneration.ownerRaw ?? "")?.token !==
            acquisitionToken
        ) {
          throw new Error(`Pi state lock ownership was lost (${name})`);
        }
        const assertOwned = async (): Promise<void> => {
          if (!(await stateLockGenerationMatches(lockPath, ownedGeneration))) {
            throw new Error(`Pi state lock ownership was lost (${name})`);
          }
        };
        const heartbeat = setInterval(() => {
          const now = new Date();
          // The token-specific path prevents an old suspended timer from
          // touching a successor's lock after its directory was renamed.
          void utimes(heartbeatFile, now, now).catch((error: unknown) => {
            if (!isCode(error, "ENOENT")) {
              this.logger.warn("failed to heartbeat Pi state lock", {
                name,
                error: errorMessage(error),
              });
            }
          });
        }, LOCK_HEARTBEAT_MS);
        heartbeat.unref();
        return {
          assertOwned,
          release: async () => {
            clearInterval(heartbeat);
            // Atomically move exactly the generation we acquired, then delete
            // the private tombstone. A successor reusing lockPath is fenced by
            // the post-rename generation check.
            await removeObservedLockGeneration(
              lockPath,
              ownedGeneration,
              "released",
            );
          },
        };
      } catch (error) {
        if (!isCode(error, "EEXIST")) throw error;
        const observed = await inspectStaleLockGeneration(
          lockPath,
          this.leaseIdentity,
        );
        if (observed) {
          // Inspect twice and require the exact directory inode + owner bytes
          // to remain stale. If the old owner released and a successor won
          // mkdir in between, the successor is never renamed or removed.
          const revalidated = await inspectStaleLockGeneration(
            lockPath,
            this.leaseIdentity,
          );
          if (
            revalidated &&
            sameStateLockGeneration(observed, revalidated) &&
            (await removeObservedStaleLock(lockPath, revalidated))
          ) {
            continue;
          }
        }
        if (Date.now() >= deadline)
          throw new Error("Timed out waiting for Pi state lock");
        await new Promise((resolvePromise) =>
          setTimeout(resolvePromise, 20 + Math.random() * 30),
        );
      }
    }
  }
}

/**
 * Persist a completed rename in the containing directory where Node supports
 * directory handles. Windows rejects opening directories through fs.open, so
 * it retains temp-file fsync plus atomic rename but cannot request this final
 * namespace flush through Node's portable filesystem API.
 */
export async function syncDirectoryEntry(
  directory: string,
  platform: NodeJS.Platform = process.platform,
): Promise<void> {
  if (platform === "win32") return;
  const directoryHandle = await open(directory, "r");
  try {
    await directoryHandle.sync();
  } finally {
    await directoryHandle.close();
  }
}

export function deriveNamespace(env: NodeJS.ProcessEnv): string {
  if (env.BUZZ_PI_NAMESPACE !== undefined && env.BUZZ_PI_NAMESPACE !== "")
    return safeNamespace(env.BUZZ_PI_NAMESPACE);
  const identity = [
    canonicalRelayIdentity(env.BUZZ_RELAY_URL),
    canonicalPrivateKeyIdentity(env.BUZZ_PRIVATE_KEY),
  ].join("\0");
  return `agent-${hash(identity).slice(0, 24)}`;
}

function canonicalRelayIdentity(value: string | undefined): string {
  if (value === undefined) return "local";
  try {
    const parsed = new URL(value);
    // Fragments never reach a WebSocket server and therefore cannot identify
    // a distinct Buzz account boundary. URL also normalizes host casing,
    // default ports, root paths, and dot segments for us.
    parsed.hash = "";
    return `url:${parsed.href}`;
  } catch {
    // The ACP harness will reject unusable relay URLs. Keeping invalid values
    // distinct here makes namespace derivation deterministic without trying
    // to guess at syntax the harness might accept in a future release.
    return `raw:${value}`;
  }
}

function canonicalPrivateKeyIdentity(value: string | undefined): string {
  if (value === undefined) return "anonymous";
  const secretKey = /^[0-9a-fA-F]{64}$/u.test(value)
    ? Uint8Array.from(Buffer.from(value, "hex"))
    : decodeNsec(value);
  if (secretKey?.byteLength !== 32) return `raw:${value}`;
  // Equivalent nsec/hex spellings now hash the same canonical key material.
  // Only the outer SHA-256 namespace is persisted; this intermediate value is
  // never written to disk or logs.
  return `secret:${Buffer.from(secretKey).toString("hex")}`;
}

const BECH32_ALPHABET = "qpzry9x8gf2tvdw0s3jn54khce6mua7l";
const BECH32_GENERATORS = [
  0x3b6a57b2, 0x26508e6d, 0x1ea119fa, 0x3d4233dd, 0x2a1462b3,
] as const;

function decodeNsec(value: string): Uint8Array | undefined {
  if (value.length < 12 || value.length > 90) return undefined;
  if (value !== value.toLowerCase() && value !== value.toUpperCase()) {
    return undefined;
  }
  const canonical = value.toLowerCase();
  const separator = canonical.lastIndexOf("1");
  if (separator !== 4 || canonical.slice(0, separator) !== "nsec") {
    return undefined;
  }
  const encoded = canonical.slice(separator + 1);
  if (encoded.length < 7) return undefined;
  const values: number[] = [];
  for (const character of encoded) {
    const index = BECH32_ALPHABET.indexOf(character);
    if (index < 0) return undefined;
    values.push(index);
  }
  const hrpValues = [
    ...Array.from("nsec", (character) => character.charCodeAt(0) >> 5),
    0,
    ...Array.from("nsec", (character) => character.charCodeAt(0) & 31),
  ];
  if (bech32Polymod([...hrpValues, ...values]) !== 1) return undefined;
  return convertFiveBitWords(values.slice(0, -6));
}

function bech32Polymod(values: readonly number[]): number {
  let checksum = 1;
  for (const value of values) {
    const top = checksum >>> 25;
    checksum = (((checksum & 0x1ffffff) << 5) ^ value) >>> 0;
    for (const [index, generator] of BECH32_GENERATORS.entries()) {
      if (((top >>> index) & 1) !== 0) {
        checksum = (checksum ^ generator) >>> 0;
      }
    }
  }
  return checksum;
}

function convertFiveBitWords(
  values: readonly number[],
): Uint8Array | undefined {
  const bytes: number[] = [];
  let accumulator = 0;
  let bits = 0;
  for (const value of values) {
    accumulator = ((accumulator << 5) | value) & 0xfff;
    bits += 5;
    while (bits >= 8) {
      bits -= 8;
      bytes.push((accumulator >>> bits) & 0xff);
    }
  }
  if (bits >= 5 || ((accumulator << (8 - bits)) & 0xff) !== 0) {
    return undefined;
  }
  return Uint8Array.from(bytes);
}

function safeNamespace(value: string): string {
  if (
    value !== value.trim() ||
    value.length < 1 ||
    value.length > 80 ||
    value === "." ||
    value === ".." ||
    !/^[a-zA-Z0-9][a-zA-Z0-9._-]*$/u.test(value)
  ) {
    throw new Error(
      "BUZZ_PI_NAMESPACE must be 1-80 characters, begin with an alphanumeric character, and contain only letters, numbers, dot, underscore, or hyphen",
    );
  }
  return value;
}

function hash(value: string): string {
  return createHash("sha256").update(value).digest("hex");
}

function newLifecycleGeneration(): string {
  return hash(`buzz-pi-lifecycle-generation-v1\0${randomUUID()}`);
}

function deriveLegacyLifecycleGeneration(piSessionId: string): string {
  validatePiSessionId(piSessionId);
  return hash(`buzz-pi-legacy-lifecycle-generation-v1\0${piSessionId}`);
}

function validateLifecycleGeneration(value: string): void {
  if (!LIFECYCLE_GENERATION_PATTERN.test(value)) {
    throw new Error("invalid lifecycle generation");
  }
}

function validateConversationId(value: string): void {
  if (value.length < 1 || value.length > 512 || hasControlCharacters(value)) {
    throw new Error("conversationId must be 1-512 characters");
  }
}

function validateResetToken(value: string | undefined): void {
  if (
    value !== undefined &&
    (value.length < 1 || value.length > 512 || hasControlCharacters(value))
  ) {
    throw new Error("resetToken must be 1-512 characters");
  }
}

function validateEventId(value: string): void {
  if (!EVENT_ID_PATTERN.test(value)) {
    throw new Error("eventId must be a lowercase UUID");
  }
}

function validateStoredSessionEvent(
  value: unknown,
): asserts value is BuzzSessionEvent {
  if (
    !isRecord(value) ||
    typeof value.type !== "string" ||
    !SESSION_EVENT_TYPES.has(value.type)
  ) {
    throw new Error("invalid pending Buzz session lifecycle event type");
  }
  requiredBoundedString(value.timestamp, "pendingEvent.timestamp", 64);
  validIsoDate(value.timestamp, "pendingEvent.timestamp");
  requiredBoundedString(value.message, "pendingEvent.message", 1_024);
  requiredBoundedString(value.piSessionId, "pendingEvent.piSessionId", 256);
  const serialized = JSON.stringify(value);
  if (Buffer.byteLength(serialized, "utf8") > 56 * 1_024) {
    throw new Error("pending Buzz session lifecycle payload is oversized");
  }
}

function isEventTemporaryFile(value: string): boolean {
  return /^\.event\.\d+\.[0-9a-f-]+\.tmp$/u.test(value);
}

function validatePiSessionId(value: string): void {
  if (value.length < 1 || value.length > 256 || hasControlCharacters(value)) {
    throw new Error("invalid Pi session id");
  }
}

function validatePathString(value: string, name: string, max: number): void {
  if (value.length < 1 || value.length > max || hasControlCharacters(value)) {
    throw new Error(`invalid ${name}`);
  }
}

function hasControlCharacters(value: string): boolean {
  for (let index = 0; index < value.length; index += 1) {
    const code = value.charCodeAt(index);
    if (code <= 0x1f || code === 0x7f) return true;
  }
  return false;
}

function validateLease(
  value: unknown,
): NonNullable<ConversationMapping["lease"]> {
  if (!isRecord(value)) throw new Error("invalid conversation lease");
  const ownerId = requiredBoundedString(value.ownerId, "lease.ownerId", 256);
  const pid = value.pid;
  if (!Number.isSafeInteger(pid) || (pid as number) <= 0) {
    throw new Error("invalid conversation lease pid");
  }
  const hostId = optionalBoundedString(value.hostId, "lease.hostId", 128);
  const bootId = optionalBoundedString(value.bootId, "lease.bootId", 128);
  if (bootId !== undefined && hostId === undefined) {
    throw new Error("invalid conversation lease boot identity");
  }
  return {
    ownerId,
    pid: pid as number,
    ...(hostId === undefined ? {} : { hostId }),
    ...(bootId === undefined ? {} : { bootId }),
    expiresAt: validIsoDate(value.expiresAt, "lease.expiresAt"),
  };
}

function requiredBoundedString(
  value: unknown,
  name: string,
  max: number,
): string {
  if (
    typeof value !== "string" ||
    value.length < 1 ||
    value.length > max ||
    hasControlCharacters(value)
  ) {
    throw new Error(`invalid ${name}`);
  }
  return value;
}

function optionalBoundedString(
  value: unknown,
  name: string,
  max: number,
): string | undefined {
  if (value === undefined) return undefined;
  return requiredBoundedString(value, name, max);
}

function requiredBoolean(value: unknown, name: string): boolean {
  if (typeof value !== "boolean") throw new Error(`invalid ${name}`);
  return value;
}

function validIsoDate(value: unknown, name: string): string {
  const raw = requiredBoundedString(value, name, 64);
  const timestamp = Date.parse(raw);
  if (!Number.isFinite(timestamp)) throw new Error(`invalid ${name}`);
  return new Date(timestamp).toISOString();
}

function sameMappingVersion(
  left: ConversationMapping,
  right: ConversationMapping,
): boolean {
  return (
    left.piSessionId === right.piSessionId &&
    left.lifecycleGeneration === right.lifecycleGeneration &&
    left.sessionFile === right.sessionFile &&
    left.lastUsedAt === right.lastUsedAt &&
    left.lastResetToken === right.lastResetToken &&
    left.relayHistoryCleared === right.relayHistoryCleared
  );
}

function emptyManifest(): ConversationManifest {
  return { version: 1, conversations: {}, resetTombstones: {} };
}

function pathIsWithin(path: string, root: string): boolean {
  const difference = relative(root, path);
  return (
    difference === "" ||
    (!difference.startsWith(`..${sep}`) &&
      difference !== ".." &&
      !isAbsolute(difference))
  );
}

function pathIsStrictlyWithin(path: string, root: string): boolean {
  return path !== root && pathIsWithin(path, root);
}

interface LockOwnerRecord {
  version: 1;
  pid: number;
  token: string;
  hostId: string;
  bootId?: string;
  createdAt: string;
}

function parseLockOwner(value: string): LockOwnerRecord | undefined {
  try {
    const owner: unknown = JSON.parse(value);
    if (
      !isRecord(owner) ||
      owner.version !== 1 ||
      !Number.isSafeInteger(owner.pid) ||
      (owner.pid as number) <= 0 ||
      typeof owner.token !== "string" ||
      owner.token.length < 1 ||
      owner.token.length > 128 ||
      typeof owner.hostId !== "string" ||
      owner.hostId.length < 1 ||
      owner.hostId.length > 128 ||
      (owner.bootId !== undefined &&
        (typeof owner.bootId !== "string" ||
          owner.bootId.length < 1 ||
          owner.bootId.length > 128)) ||
      typeof owner.createdAt !== "string" ||
      !Number.isFinite(Date.parse(owner.createdAt))
    ) {
      return undefined;
    }
    return {
      version: 1,
      pid: owner.pid as number,
      token: owner.token,
      hostId: owner.hostId,
      ...(owner.bootId === undefined ? {} : { bootId: owner.bootId }),
      createdAt: new Date(Date.parse(owner.createdAt)).toISOString(),
    };
  } catch {
    // Legacy PID/token owners and malformed records receive conservative TTL
    // handling instead of a local PID probe.
    return undefined;
  }
}

async function inspectStaleLockGeneration(
  path: string,
  localIdentity: LeaseProcessIdentity,
): Promise<StateLockGeneration | undefined> {
  const generation = await captureStateLockGeneration(path);
  if (!generation) return undefined;
  const ownerRaw = generation.ownerRaw;
  if (ownerRaw === undefined) {
    // mkdir() wins the lock before its owner file is written. A vanished
    // directory means release won the race; it authorizes a retry, never
    // removal of whatever generation may appear at the same path next.
    return Date.now() - generation.mtimeMs > LOCK_OWNER_WRITE_GRACE_MS
      ? generation
      : undefined;
  }
  try {
    const owner = parseLockOwner(ownerRaw);
    const metadata = owner
      ? await stat(join(path, `heartbeat-${owner.token}`)).catch(
          async (error: unknown) => {
            if (!isCode(error, "ENOENT")) throw error;
            // Owner-to-heartbeat creation has a bounded setup window; the
            // pre-observed directory mtime excludes any successor generation.
            return { mtimeMs: generation.mtimeMs };
          },
        )
      : { mtimeMs: generation.mtimeMs };
    const heartbeatExpired =
      Date.now() - metadata.mtimeMs > LOCK_FOREIGN_STALE_MS;
    if (!owner) return heartbeatExpired ? generation : undefined;
    if (owner.hostId !== localIdentity.hostId)
      return heartbeatExpired ? generation : undefined;
    if (!localIdentity.pidProbeSafe)
      return heartbeatExpired ? generation : undefined;
    if (
      owner.bootId !== undefined &&
      localIdentity.bootId !== undefined &&
      owner.bootId !== localIdentity.bootId
    ) {
      return generation;
    }
    // Never steal from a confirmed-live process in the same PID domain. Pi's
    // session initialization can append to an existing JSONL while resolve()
    // holds this lock; a machine suspension longer than the heartbeat TTL is
    // not proof that those writes stopped. Dead PIDs and prior boots are safe
    // to recover immediately. Foreign/unprovable owners use the conservative
    // heartbeat path above and always get fresh JSONL takeover semantics.
    return isProcessAlive(owner.pid) ? undefined : generation;
  } catch {
    return undefined;
  }
}

/** @internal Exposed only for deterministic lock-generation regression tests. */
export async function captureStateLockGeneration(
  path: string,
): Promise<StateLockGeneration | undefined> {
  let before: Awaited<ReturnType<typeof stat>>;
  try {
    before = await stat(path);
  } catch (error) {
    if (isCode(error, "ENOENT")) return undefined;
    throw error;
  }
  let ownerRaw: string | undefined;
  try {
    ownerRaw = await readFile(join(path, "owner"), "utf8");
  } catch (error) {
    if (!isCode(error, "ENOENT")) throw error;
  }
  let after: Awaited<ReturnType<typeof stat>>;
  try {
    after = await stat(path);
  } catch (error) {
    if (isCode(error, "ENOENT")) return undefined;
    throw error;
  }
  if (!sameStateLockIdentity(before, after)) return undefined;
  return {
    device: after.dev,
    inode: after.ino,
    birthtimeMs: after.birthtimeMs,
    mtimeMs: after.mtimeMs,
    ownerRaw,
  };
}

async function stateLockGenerationMatches(
  path: string,
  expected: StateLockGeneration,
): Promise<boolean> {
  const current = await captureStateLockGeneration(path);
  return current !== undefined && sameStateLockGeneration(current, expected);
}

function sameStateLockGeneration(
  left: StateLockGeneration,
  right: StateLockGeneration,
): boolean {
  return (
    left.device === right.device &&
    left.inode === right.inode &&
    left.birthtimeMs === right.birthtimeMs &&
    left.ownerRaw === right.ownerRaw
  );
}

function sameStateLockIdentity(
  left: Awaited<ReturnType<typeof stat>>,
  right: Awaited<ReturnType<typeof stat>>,
): boolean {
  return (
    left.dev === right.dev &&
    left.ino === right.ino &&
    left.birthtimeMs === right.birthtimeMs
  );
}

/** @internal Exposed only for deterministic lock-generation regression tests. */
export async function removeObservedStaleLock(
  path: string,
  expected: StateLockGeneration,
): Promise<boolean> {
  return removeObservedLockGeneration(path, expected, "stale");
}

async function removeObservedLockGeneration(
  path: string,
  expected: StateLockGeneration,
  disposition: "released" | "stale",
): Promise<boolean> {
  if (!(await stateLockGenerationMatches(path, expected))) return false;
  const retiredPath = `${path}.${disposition}-${randomUUID()}`;
  try {
    await rename(path, retiredPath);
  } catch (error) {
    if (isCode(error, "ENOENT")) return false;
    throw error;
  }
  if (!(await stateLockGenerationMatches(retiredPath, expected))) {
    // Never delete a successor that won the pathname after inspection. Try to
    // restore it; if another owner already filled the path, preserve both and
    // fail closed rather than deleting an unrecognized generation.
    await rename(retiredPath, path).catch(() => {});
    throw new Error("Pi state lock generation changed during removal");
  }
  await rm(retiredPath, { recursive: true, force: true });
  return true;
}

function isProcessAlive(pid: number): boolean {
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    return isCode(error, "EPERM");
  }
}

function discoverLocalLeaseIdentity(): LeaseProcessIdentity {
  const machineIdentity = readMachineIdentity();
  const processDomainIdentity = readProcessDomainIdentity();
  const pidProbeSafe =
    machineIdentity !== undefined && processDomainIdentity !== undefined;
  const hostIdentity = pidProbeSafe
    ? `machine\0${machineIdentity}\0process-domain\0${processDomainIdentity}`
    : `hostname-fallback\0${hostname()}`;
  const bootIdentity = readBootIdentity();
  return {
    hostId: hashIdentity(hostIdentity),
    ...(bootIdentity === undefined
      ? {}
      : { bootId: hashIdentity(`boot\0${bootIdentity}`) }),
    pidProbeSafe,
  };
}

function readMachineIdentity(): string | undefined {
  if (process.platform === "linux") {
    for (const path of ["/etc/machine-id", "/var/lib/dbus/machine-id"]) {
      try {
        const value = readFileSync(path, "utf8").trim();
        if (value.length > 0) return `linux:${value}`;
      } catch {
        // Try the next standard location.
      }
    }
    return undefined;
  }
  if (process.platform === "darwin") {
    try {
      const value = execFileSync(
        "/usr/sbin/ioreg",
        ["-rd1", "-c", "IOPlatformExpertDevice"],
        {
          encoding: "utf8",
          stdio: ["ignore", "pipe", "ignore"],
          timeout: 1_000,
          maxBuffer: 64 * 1_024,
        },
      );
      const uuid = /"IOPlatformUUID"\s*=\s*"([^"]+)"/u.exec(value)?.[1];
      if (uuid) return `darwin:${uuid}`;
    } catch {
      // Conservative hostname-only fallback below cannot authorize PID probes.
    }
    return undefined;
  }
  if (process.platform === "win32") {
    try {
      const value = execFileSync(
        "reg.exe",
        [
          "query",
          "HKLM\\SOFTWARE\\Microsoft\\Cryptography",
          "/v",
          "MachineGuid",
        ],
        {
          encoding: "utf8",
          stdio: ["ignore", "pipe", "ignore"],
          timeout: 1_000,
          maxBuffer: 64 * 1_024,
        },
      );
      const guid = /MachineGuid\s+REG_SZ\s+(\S+)/u.exec(value)?.[1];
      if (guid) return `windows:${guid}`;
    } catch {
      // Conservative hostname-only fallback below cannot authorize PID probes.
    }
  }
  return undefined;
}

function readProcessDomainIdentity(): string | undefined {
  if (process.platform === "linux") {
    try {
      return `linux-pidns:${readlinkSync("/proc/self/ns/pid")}`;
    } catch {
      return undefined;
    }
  }
  // macOS and Windows expose host-global process identifiers.
  if (process.platform === "darwin" || process.platform === "win32") {
    return `${process.platform}:global-pid-domain`;
  }
  return undefined;
}

function readBootIdentity(): string | undefined {
  if (process.platform === "linux") {
    try {
      const value = readFileSync(
        "/proc/sys/kernel/random/boot_id",
        "utf8",
      ).trim();
      if (value.length > 0) return `linux:${value}`;
    } catch {
      // Fall through to an unknown boot. Same-host dead PIDs remain safely
      // recoverable; live PIDs are then bounded by the advisory lease TTL.
    }
  }
  if (process.platform === "darwin") {
    try {
      const value = execFileSync("/usr/sbin/sysctl", ["-n", "kern.boottime"], {
        encoding: "utf8",
        stdio: ["ignore", "pipe", "ignore"],
        timeout: 1_000,
        maxBuffer: 4_096,
      });
      const seconds = /sec\s*=\s*(\d+)/u.exec(value)?.[1];
      if (seconds !== undefined) return `darwin:${seconds}`;
    } catch {
      // See the Linux fallback comment above.
    }
  }
  return undefined;
}

function hashIdentity(value: string): string {
  return createHash("sha256").update(value).digest("hex");
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isCode(error: unknown, code: string): boolean {
  return (
    typeof error === "object" &&
    error !== null &&
    "code" in error &&
    error.code === code
  );
}

function isRecoverableStaleSessionError(error: unknown): boolean {
  if (isCode(error, "ENOENT")) return true;
  const message = errorMessage(error);
  // Keep this intentionally narrow. Provider/auth/extension/quota failures are
  // environmental, not proof that the persisted transcript is stale; replacing
  // its durable route on those errors would silently discard thread context.
  return message.startsWith("Session file is not a valid pi session:");
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
