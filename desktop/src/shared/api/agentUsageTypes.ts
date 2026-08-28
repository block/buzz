export type AgentUsageSummary = {
  pubkey: string;
  name: string;
  model: string | null;
  parallelism: number;
  isRunning: boolean;
  promptCount: number;
  promptBytes: number;
  /** Approximation only: measured prompt UTF-8 bytes divided by four. */
  estimatedPromptTokens: number;
  peakPromptBytes: number;
  sessionStartCount: number;
  largePromptCount: number;
  retryCount: number;
  quotaLimitCount: number;
};
