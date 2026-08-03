import type { Writable } from "node:stream";

export type JsonRpcId = string | number | null;

export interface JsonRpcRequest {
  jsonrpc: "2.0";
  id: JsonRpcId;
  method: string;
  params?: unknown;
}

export interface JsonRpcNotification {
  jsonrpc: "2.0";
  method: string;
  params?: unknown;
}

export type JsonRpcInbound = JsonRpcRequest | JsonRpcNotification;

export interface AcpTextBlock {
  type: "text";
  text: string;
}

export interface AcpImageBlock {
  type: "image";
  data: string;
  mimeType: string;
}

export type AcpContentBlock = AcpTextBlock | AcpImageBlock;

export interface SessionNewParams {
  cwd: string;
  mcpServers?: unknown[];
  systemPrompt?: string;
  _meta?: {
    sessionTitle?: string;
    [key: string]: unknown;
  };
}

export interface SessionPromptParams {
  sessionId: string;
  prompt: AcpContentBlock[];
}

export interface SessionIdParams {
  sessionId: string;
}

export interface SessionSetModelParams extends SessionIdParams {
  modelId: string;
}

export interface SessionSetConfigOptionParams extends SessionIdParams {
  configId: string;
  value: string;
}

export interface SessionSteeringParams extends SessionIdParams {
  prompt: AcpContentBlock[];
}

export interface ModelDescriptor {
  id: string;
  name: string;
  description?: string;
}

export interface ContextSnapshot {
  usedTokens: number | null;
  limitTokens: number;
  effectiveLimitTokens: number;
  compactionThresholdTokens: number;
  autoCompaction: boolean;
  compacting: boolean;
  model: string | null;
  thinkingLevel: string;
  piSessionId: string;
}

export interface ResourceSnapshot {
  extensions: number;
  skills: number;
  prompts: number;
  contextFiles: number;
  errors: string[];
  projectTrusted: boolean;
  commands: SlashCommandDescriptor[];
}

export interface SlashCommandDescriptor {
  name: string;
  description: string;
}

export interface SessionUsageSnapshot {
  contextTokens: number | null;
  accumulatedInputTokens: number;
  accumulatedOutputTokens: number;
  accumulatedCachedInputTokens: number;
  accumulatedCost: number | null;
  model: string | null;
}

export type CompactionReason =
  | "manual"
  | "threshold"
  | "overflow"
  | "preflight";

export type BuzzSessionEvent =
  | ({
      type: "compaction_completed";
      compactionId: string;
      reason: CompactionReason;
      beforeTokens: number | null;
      afterTokens: number | null;
      limitTokens: number;
      effectiveLimitTokens: number;
      compactionThresholdTokens: number;
      willRetry: boolean;
      fromExtension: boolean;
    } & CommonBuzzSessionEvent)
  | ({
      type: "compaction_failed";
      compactionId: string;
      reason: CompactionReason;
      beforeTokens: number | null;
      limitTokens: number;
      effectiveLimitTokens: number;
      compactionThresholdTokens: number;
      error: string;
      aborted: boolean;
      willRetry: boolean;
      fromExtension: boolean;
    } & CommonBuzzSessionEvent)
  | ({
      type: "context_status";
      usedTokens: number | null;
      remainingTokens: number | null;
      percent: number | null;
      limitTokens: number;
      effectiveLimitTokens: number;
      compactionThresholdTokens: number;
      autoCompaction: boolean;
      compacting: boolean;
      model: string | null;
    } & CommonBuzzSessionEvent)
  | ({
      type: "session_reset";
      previousPiSessionId: string;
      limitTokens: number;
      effectiveLimitTokens: number;
      compactionThresholdTokens: number;
    } & CommonBuzzSessionEvent)
  | ({
      type: "extensions_reloaded";
      extensions: number;
      skills: number;
      prompts: number;
      contextFiles: number;
      errors: string[];
      projectTrusted: boolean;
    } & CommonBuzzSessionEvent);

interface CommonBuzzSessionEvent {
  timestamp: string;
  message: string;
  piSessionId: string;
}

export interface PendingBuzzSessionEvent {
  conversationId: string;
  eventId: string;
  /** Durable context epoch; Pi session IDs may change within one epoch. */
  lifecycleGeneration: string;
  event: BuzzSessionEvent;
  createdAt: string;
}

export interface AdapterEventSink {
  sessionUpdate(sessionId: string, update: Record<string, unknown>): void;
  buzzSessionEvent(
    sessionId: string,
    event: BuzzSessionEvent,
    deliveryId?: string,
  ): void | Promise<void>;
  usageUpdate(
    sessionId: string,
    usage: SessionUsageSnapshot,
    contextLimit: number,
  ): void;
}

export interface AgentSessionHandle {
  readonly piSessionId: string;
  readonly sessionFile: string | undefined;
  readonly cwd: string;
  readonly isBusy: boolean;
  readonly isValid: boolean;
  prompt(
    text: string,
    images?: AcpImageBlock[],
  ): Promise<"end_turn" | "cancelled" | "max_tokens">;
  steer(text: string): Promise<void>;
  abort(): Promise<void>;
  setModel(modelId: string): Promise<void>;
  setThinkingLevel(level: string): Promise<void>;
  reload(): Promise<ResourceSnapshot>;
  reset(): Promise<{
    previousPiSessionId: string;
    resources: ResourceSnapshot;
  }>;
  getModels(): ModelDescriptor[];
  getThinkingLevels(): string[];
  getResources(): ResourceSnapshot;
  getContextSnapshot(): ContextSnapshot;
  getUsageSnapshot(): SessionUsageSnapshot;
  /** Replay child-side durable lifecycle notices after outer routing is live. */
  replayLifecycleEvents?(): Promise<void>;
  /** Mark one child-side lifecycle notice durable in the parent adapter. */
  acknowledgeLifecycleEvent?(deliveryId: string): Promise<void>;
  dispose(): Promise<void>;
}

export interface CreateSessionOptions {
  cwd: string;
  /** Original lexical workspace path when cwd has already been canonicalized. */
  requestedCwd?: string;
  systemPrompt?: string;
  title?: string;
  persistedSessionFile?: string;
  eventSink: AdapterEventSink;
  acpSessionId: string;
}

export interface AgentSessionFactory {
  create(options: CreateSessionOptions): Promise<AgentSessionHandle>;
  setInvalidationHandler?(
    handler: (
      sessionIds: readonly string[],
      error: Error,
    ) => void | Promise<void>,
  ): void;
}

export interface SessionMetadata {
  sessionId: string;
  cwd: string;
  systemPrompt?: string;
  title?: string;
  piSessionId: string;
  sessionFile?: string;
  createdAt: string;
  lastUsedAt: string;
}

export interface ConversationMapping {
  conversationId: string;
  cwd: string;
  sessionFile: string;
  piSessionId: string;
  /** Rotates only at an authenticated Buzz reset boundary. */
  lifecycleGeneration: string;
  createdAt: string;
  lastUsedAt: string;
  lastResetToken?: string;
  relayHistoryCleared?: boolean;
  lease?: {
    ownerId: string;
    pid: number;
    hostId?: string;
    bootId?: string;
    expiresAt: string;
  };
}

export interface Logger {
  debug(message: string, fields?: Record<string, unknown>): void;
  info(message: string, fields?: Record<string, unknown>): void;
  warn(message: string, fields?: Record<string, unknown>): void;
  error(message: string, fields?: Record<string, unknown>): void;
}

export interface OutputWriter {
  write(value: unknown): void;
  end(): Promise<void>;
}

export type RawStdoutWrite = Writable["write"];
