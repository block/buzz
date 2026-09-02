import { z } from "zod";

import type {
  ProjectCanvasCapability,
  ProjectCanvasChannelSummary,
  ProjectCanvasDataState,
  ProjectCanvasProjectSummary,
  ProjectCanvasRpcError,
  ProjectCanvasRpcErrorCode,
} from "./projectCanvasProtocol";

/**
 * Data broker for widget-initiated canvas RPC. The broker owns every scope
 * decision: widgets never supply repo addresses or channel ids that widen the
 * data they can see — all sources are bound to the hosting project by the
 * page that feeds `setSources`, and commands/opens are resolved against those
 * same sources before any delegate runs.
 */

export const PROJECT_CANVAS_QUERY_ROW_LIMIT = 50;
export const PROJECT_CANVAS_LOOKUP_PUBKEY_LIMIT = 32;
export const PROJECT_CANVAS_SEARCH_QUERY_MAX_LENGTH = 64;
export const PROJECT_CANVAS_DM_MESSAGE_MAX_LENGTH = 2_000;

export type ProjectCanvasTaskRow = {
  assignees: string[];
  category: string;
  commentCount: number;
  displayId: string;
  id: string;
  status: string;
  title: string;
  updatedAt: number;
};

export type ProjectCanvasReviewListRow = {
  author: string;
  branch: string | null;
  displayId: string;
  id: string;
  status: "Open" | "Draft" | "Merged" | "Closed";
  title: string;
  updatedAt: number;
};

export type ProjectCanvasPersonRow = {
  avatarDataUrl: string | null;
  displayName: string | null;
  isAgent: boolean;
  pubkey: string;
};

export type ProjectCanvasBrokerSources = {
  channels: ProjectCanvasDataState<ProjectCanvasChannelSummary[]>;
  project: ProjectCanvasDataState<ProjectCanvasProjectSummary>;
  reviews: ProjectCanvasDataState<ProjectCanvasReviewListRow[]>;
  tasks: ProjectCanvasDataState<ProjectCanvasTaskRow[]>;
};

export type ProjectCanvasTaskCommand =
  | {
      name: "tasks.setStatus";
      status: "open" | "done" | "closed" | "draft";
      task: ProjectCanvasTaskRow;
    }
  | {
      name: "tasks.assign" | "tasks.unassign";
      assignee: string | null;
      task: ProjectCanvasTaskRow;
    };

export type ProjectCanvasOpenTarget =
  | { type: "channel"; id: string }
  | { type: "user"; pubkey: string }
  | { type: "task"; id: string }
  | { type: "review"; id: string };

export type ProjectCanvasBrokerDelegate = {
  lookupPeople: (pubkeys: string[]) => Promise<ProjectCanvasPersonRow[]>;
  searchPeople: (
    query: string,
    limit: number,
  ) => Promise<ProjectCanvasPersonRow[]>;
  runTaskCommand: (command: ProjectCanvasTaskCommand) => Promise<void>;
  openTarget: (target: ProjectCanvasOpenTarget) => Promise<void>;
  sendDirectMessage: (recipient: string, message: string) => Promise<void>;
};

export type ProjectCanvasQueryResult = {
  data: unknown;
  status: "loading" | "ready" | "error";
};

export class ProjectCanvasBrokerError extends Error {
  readonly code: ProjectCanvasRpcErrorCode;

  constructor(code: ProjectCanvasRpcErrorCode, message: string) {
    super(message);
    this.code = code;
  }

  toRpcError(): ProjectCanvasRpcError {
    return { code: this.code, message: this.message };
  }
}

const EMPTY_SOURCES: ProjectCanvasBrokerSources = {
  channels: { data: null, status: "loading" },
  project: { data: null, status: "loading" },
  reviews: { data: null, status: "loading" },
  tasks: { data: null, status: "loading" },
};

const hexIdSchema = z.string().regex(/^[0-9a-f]{64}$/i);
const limitSchema = z
  .number()
  .int()
  .min(1)
  .max(PROJECT_CANVAS_QUERY_ROW_LIMIT)
  .optional();

const channelsParamsSchema = z
  .object({
    limit: limitSchema,
    relationship: z.enum(["home", "related"]).optional(),
  })
  .strict();
const reviewsParamsSchema = z
  .object({
    author: hexIdSchema.optional(),
    limit: limitSchema,
    status: z.enum(["Open", "Draft", "Merged", "Closed"]).optional(),
  })
  .strict();
const tasksParamsSchema = z
  .object({
    assignee: hexIdSchema.optional(),
    limit: limitSchema,
    status: z
      .enum(["Triage", "Backlog", "In Progress", "In Review", "Done", "Closed"])
      .optional(),
  })
  .strict();
const taskGetParamsSchema = z.object({ id: hexIdSchema }).strict();
const lookupParamsSchema = z
  .object({
    pubkeys: z
      .array(hexIdSchema)
      .min(1)
      .max(PROJECT_CANVAS_LOOKUP_PUBKEY_LIMIT),
  })
  .strict();
const searchParamsSchema = z
  .object({
    limit: limitSchema,
    query: z.string().min(1).max(PROJECT_CANVAS_SEARCH_QUERY_MAX_LENGTH),
  })
  .strict();
const setStatusParamsSchema = z
  .object({
    id: hexIdSchema,
    status: z.enum(["open", "done", "closed", "draft"]),
  })
  .strict();
const assignParamsSchema = z
  .object({
    assignee: hexIdSchema.optional(),
    id: hexIdSchema,
  })
  .strict();
const dmSendParamsSchema = z
  .object({
    message: z.string().min(1).max(PROJECT_CANVAS_DM_MESSAGE_MAX_LENGTH),
    pubkey: hexIdSchema,
  })
  .strict();
const openTargetSchema = z.discriminatedUnion("type", [
  z
    .object({ id: z.string().min(1).max(256), type: z.literal("channel") })
    .strict(),
  z.object({ pubkey: hexIdSchema, type: z.literal("user") }).strict(),
  z.object({ id: hexIdSchema, type: z.literal("task") }).strict(),
  z.object({ id: hexIdSchema, type: z.literal("review") }).strict(),
]);

const QUERY_CAPABILITIES: Record<string, ProjectCanvasCapability> = {
  "people.lookup": "project.people.read",
  "people.search": "project.people.read",
  "project.channels.list": "project.channels.read",
  "project.metadata": "project.metadata.read",
  "project.reviews.list": "project.reviews.read",
  "project.tasks.get": "project.tasks.read",
  "project.tasks.list": "project.tasks.read",
};

const SUBSCRIBABLE_QUERIES = new Set([
  "project.channels.list",
  "project.metadata",
  "project.reviews.list",
  "project.tasks.list",
]);

function invalidParams(issue: string): ProjectCanvasBrokerError {
  return new ProjectCanvasBrokerError("invalid-params", issue);
}

function parseParams<T>(schema: z.ZodType<T>, params: unknown): T {
  const parsed = schema.safeParse(params ?? {});
  if (!parsed.success) {
    throw invalidParams(
      parsed.error.issues[0]?.message ?? "Invalid query parameters.",
    );
  }
  return parsed.data;
}

function requireCapability(
  name: string,
  capabilities: readonly ProjectCanvasCapability[],
): void {
  const required = QUERY_CAPABILITIES[name];
  if (!required) {
    throw new ProjectCanvasBrokerError(
      "unsupported",
      `Unknown canvas query: ${name}`,
    );
  }
  if (!capabilities.includes(required)) {
    throw new ProjectCanvasBrokerError(
      "forbidden",
      `Canvas query ${name} requires the ${required} capability.`,
    );
  }
}

type LiveSubscription = {
  lastSerialized: string | null;
  name: string;
  onUpdate: (result: ProjectCanvasQueryResult) => void;
  params: unknown;
};

export class ProjectCanvasBroker {
  private readonly delegate: ProjectCanvasBrokerDelegate;
  private sources: ProjectCanvasBrokerSources = EMPTY_SOURCES;
  private readonly subscriptions = new Set<LiveSubscription>();

  constructor(delegate: ProjectCanvasBrokerDelegate) {
    this.delegate = delegate;
  }

  setSources(sources: ProjectCanvasBrokerSources): void {
    this.sources = sources;
    for (const subscription of this.subscriptions) {
      const result = this.resolveProjectQuery(
        subscription.name,
        subscription.params,
      );
      const serialized = JSON.stringify(result);
      if (serialized === subscription.lastSerialized) continue;
      subscription.lastSerialized = serialized;
      subscription.onUpdate(result);
    }
  }

  async query(
    name: string,
    params: unknown,
    capabilities: readonly ProjectCanvasCapability[],
  ): Promise<ProjectCanvasQueryResult> {
    requireCapability(name, capabilities);
    if (name === "people.lookup") {
      const parsed = parseParams(lookupParamsSchema, params);
      const pubkeys = [
        ...new Set(parsed.pubkeys.map((pubkey) => pubkey.toLowerCase())),
      ];
      const rows = await this.delegate.lookupPeople(pubkeys);
      return {
        data: rows.slice(0, PROJECT_CANVAS_QUERY_ROW_LIMIT),
        status: "ready",
      };
    }
    if (name === "people.search") {
      const parsed = parseParams(searchParamsSchema, params);
      const rows = await this.delegate.searchPeople(
        parsed.query,
        parsed.limit ?? 8,
      );
      return {
        data: rows.slice(0, parsed.limit ?? PROJECT_CANVAS_QUERY_ROW_LIMIT),
        status: "ready",
      };
    }
    return this.resolveProjectQuery(name, params);
  }

  /**
   * Live variant of {@link query} for the project-scoped datasets. Pushes an
   * immediate result, then again whenever `setSources` changes the outcome.
   * People queries are one-shot only.
   */
  subscribe(
    name: string,
    params: unknown,
    capabilities: readonly ProjectCanvasCapability[],
    onUpdate: (result: ProjectCanvasQueryResult) => void,
  ): () => void {
    requireCapability(name, capabilities);
    if (!SUBSCRIBABLE_QUERIES.has(name)) {
      throw new ProjectCanvasBrokerError(
        "unsupported",
        `Canvas query ${name} does not support live subscriptions.`,
      );
    }
    const initial = this.resolveProjectQuery(name, params);
    const subscription: LiveSubscription = {
      lastSerialized: JSON.stringify(initial),
      name,
      onUpdate,
      params,
    };
    this.subscriptions.add(subscription);
    onUpdate(initial);
    return () => {
      this.subscriptions.delete(subscription);
    };
  }

  async command(
    name: string,
    params: unknown,
    capabilities: readonly ProjectCanvasCapability[],
  ): Promise<void> {
    if (name === "dm.send") {
      if (!capabilities.includes("app.dm.send")) {
        throw new ProjectCanvasBrokerError(
          "forbidden",
          "Canvas command dm.send requires the app.dm.send capability.",
        );
      }
      const parsed = parseParams(dmSendParamsSchema, params);
      const message = parsed.message.trim();
      if (!message) {
        throw invalidParams("Direct message content must not be blank.");
      }
      // Like the `user` open target, the recipient is any valid pubkey (it
      // legitimately comes from people.lookup/people.search results, which
      // are not host sources). Containment is the consent-gated capability,
      // the command rate budget, and the host toast on every send.
      await this.delegate.sendDirectMessage(
        parsed.pubkey.toLowerCase(),
        message,
      );
      return;
    }
    if (
      name !== "tasks.setStatus" &&
      name !== "tasks.assign" &&
      name !== "tasks.unassign"
    ) {
      throw new ProjectCanvasBrokerError(
        "unsupported",
        `Unknown canvas command: ${name}`,
      );
    }
    if (!capabilities.includes("project.tasks.write")) {
      throw new ProjectCanvasBrokerError(
        "forbidden",
        `Canvas command ${name} requires the project.tasks.write capability.`,
      );
    }
    if (name === "tasks.setStatus") {
      const parsed = parseParams(setStatusParamsSchema, params);
      await this.delegate.runTaskCommand({
        name,
        status: parsed.status,
        task: this.requireTask(parsed.id),
      });
      return;
    }
    const parsed = parseParams(assignParamsSchema, params);
    await this.delegate.runTaskCommand({
      assignee: parsed.assignee?.toLowerCase() ?? null,
      name,
      task: this.requireTask(parsed.id),
    });
  }

  async open(
    target: unknown,
    capabilities: readonly ProjectCanvasCapability[],
  ): Promise<void> {
    if (!capabilities.includes("app.open")) {
      throw new ProjectCanvasBrokerError(
        "forbidden",
        "Canvas navigation requires the app.open capability.",
      );
    }
    const parsed = openTargetSchema.safeParse(target);
    if (!parsed.success) {
      throw invalidParams("Invalid canvas navigation target.");
    }
    const resolved = parsed.data;
    if (resolved.type === "channel") {
      const channels = this.sources.channels.data ?? [];
      if (!channels.some((channel) => channel.id === resolved.id)) {
        throw new ProjectCanvasBrokerError(
          "not-found",
          "Canvas navigation targets must be channels bound to this project.",
        );
      }
    }
    if (resolved.type === "task") {
      this.requireTask(resolved.id);
    }
    if (resolved.type === "review") {
      const reviews = this.sources.reviews.data ?? [];
      if (
        !reviews.some(
          (review) => review.id.toLowerCase() === resolved.id.toLowerCase(),
        )
      ) {
        throw new ProjectCanvasBrokerError(
          "not-found",
          "Canvas navigation targets must be reviews in this project.",
        );
      }
    }
    await this.delegate.openTarget(
      resolved.type === "user"
        ? { ...resolved, pubkey: resolved.pubkey.toLowerCase() }
        : resolved,
    );
  }

  private requireTask(id: string): ProjectCanvasTaskRow {
    const task = (this.sources.tasks.data ?? []).find(
      (candidate) => candidate.id.toLowerCase() === id.toLowerCase(),
    );
    if (!task) {
      throw new ProjectCanvasBrokerError(
        "not-found",
        "Canvas task operations must reference a task in this project.",
      );
    }
    return task;
  }

  private resolveProjectQuery(
    name: string,
    params: unknown,
  ): ProjectCanvasQueryResult {
    if (name === "project.metadata") {
      parseParams(z.object({}).strict(), params);
      return { ...this.sources.project };
    }
    if (name === "project.channels.list") {
      const parsed = parseParams(channelsParamsSchema, params);
      return this.listResult(this.sources.channels, (rows) =>
        rows
          .filter(
            (row) =>
              !parsed.relationship || row.relationship === parsed.relationship,
          )
          .slice(0, parsed.limit ?? PROJECT_CANVAS_QUERY_ROW_LIMIT),
      );
    }
    if (name === "project.reviews.list") {
      const parsed = parseParams(reviewsParamsSchema, params);
      const author = parsed.author?.toLowerCase();
      return this.listResult(this.sources.reviews, (rows) =>
        rows
          .filter(
            (row) =>
              (!parsed.status || row.status === parsed.status) &&
              (!author || row.author.toLowerCase() === author),
          )
          .slice(0, parsed.limit ?? PROJECT_CANVAS_QUERY_ROW_LIMIT),
      );
    }
    if (name === "project.tasks.list") {
      const parsed = parseParams(tasksParamsSchema, params);
      const assignee = parsed.assignee?.toLowerCase();
      return this.listResult(this.sources.tasks, (rows) =>
        rows
          .filter(
            (row) =>
              (!parsed.status || row.status === parsed.status) &&
              (!assignee ||
                row.assignees.some(
                  (candidate) => candidate.toLowerCase() === assignee,
                )),
          )
          .slice(0, parsed.limit ?? PROJECT_CANVAS_QUERY_ROW_LIMIT),
      );
    }
    if (name === "project.tasks.get") {
      const parsed = parseParams(taskGetParamsSchema, params);
      if (this.sources.tasks.status !== "ready") {
        return { data: null, status: this.sources.tasks.status };
      }
      return { data: this.requireTask(parsed.id), status: "ready" };
    }
    throw new ProjectCanvasBrokerError(
      "unsupported",
      `Unknown canvas query: ${name}`,
    );
  }

  private listResult<T>(
    source: ProjectCanvasDataState<T[]>,
    select: (rows: T[]) => T[],
  ): ProjectCanvasQueryResult {
    if (source.status !== "ready") {
      return { data: null, status: source.status };
    }
    return { data: select(source.data ?? []), status: "ready" };
  }
}
