export type WorkflowStatus = "active" | "disabled" | "archived";

export type Workflow = {
  id: string;
  name: string;
  ownerPubkey: string;
  channelId: string | null;
  definition: Record<string, unknown>;
  status: WorkflowStatus;
  createdAt: number;
  updatedAt: number;
};

export type WorkflowSaveResult = {
  workflow: Workflow;
  webhookSecret: string | null;
};

export type WorkflowRunStatus =
  | "pending"
  | "running"
  | "completed"
  | "failed"
  | "cancelled"
  | "waiting_approval";

export type TraceEntry = {
  stepId: string;
  status: string;
  output: Record<string, unknown>;
  startedAt: number | null;
  completedAt: number | null;
  error: string | null;
};

export type WorkflowRun = {
  id: string;
  workflowId: string;
  status: WorkflowRunStatus;
  currentStep: number | null;
  executionTrace: TraceEntry[];
  startedAt: number | null;
  completedAt: number | null;
  errorMessage: string | null;
  createdAt: number;
};

export type WorkflowApprovalStatus =
  | "pending"
  | "granted"
  | "denied"
  | "expired";

export type WorkflowApproval = {
  token: string;
  workflowId: string;
  runId: string;
  stepId: string;
  stepIndex: number;
  approverSpec: string;
  status: WorkflowApprovalStatus;
  approverPubkey: string | null;
  /** The approver's own comment, travelling back with the decision. */
  note: string | null;
  /**
   * The exact prompt the approver is being asked to approve. `null` for gates
   * created before the relay recorded it — render that as "not recorded", never
   * as an empty package, or an unreviewable gate looks like a reviewed one.
   */
  requestMessage: string | null;
  expiresAt: string;
  createdAt: number;
};

export type TriggerWorkflowResponse = {
  runId: string;
  workflowId: string;
  status: string;
};

export type ApprovalActionResponse = {
  token: string;
  status: string;
  runId: string;
  workflowId: string;
};
