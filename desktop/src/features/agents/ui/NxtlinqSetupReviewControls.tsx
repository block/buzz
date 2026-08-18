import type * as React from "react";
import {
  CheckCircle2,
  FolderOpen,
  KeyRound,
  LoaderCircle,
  Sparkles,
  TriangleAlert,
} from "lucide-react";

import type { NxtlinqPolicyDraft } from "../agentManagement";
import { DiffViewer } from "@/features/messages/ui/DiffViewer";
import {
  pickNxtlinqDirectory,
  pickNxtlinqTrustStore,
  type NxtlinqAttestInitializationStatus,
  type NxtlinqManifestPreview,
  type NxtlinqManifestSignResult,
} from "@/shared/api/tauriNxtlinq";
import { Button } from "@/shared/ui/button";
import { Checkbox } from "@/shared/ui/checkbox";
import { Input } from "@/shared/ui/input";
import { Textarea } from "@/shared/ui/textarea";
import { formatNxtlinqPolicyDraft } from "./nxtlinqPolicyDraft";

export type NxtlinqSetupStep = "workspace" | "trust" | "policy" | "activate";

export const NXTLINQ_SETUP_STEPS: ReadonlyArray<{
  id: NxtlinqSetupStep;
  label: string;
}> = [
  { id: "workspace", label: "Project" },
  { id: "trust", label: "Local trust" },
  { id: "policy", label: "Policy" },
  { id: "activate", label: "Activate" },
];

export function NxtlinqSetupProgress({
  activeStep,
  steps,
}: {
  activeStep: NxtlinqSetupStep;
  steps: typeof NXTLINQ_SETUP_STEPS;
}) {
  const activeIndex = steps.findIndex((step) => step.id === activeStep);
  return (
    <ol
      aria-label="Nxtlinq setup progress"
      className="grid shrink-0 border-b border-border/60 px-6"
      data-testid="nxtlinq-setup-progress"
      style={{ gridTemplateColumns: `repeat(${steps.length}, minmax(0, 1fr))` }}
    >
      {steps.map((step, index) => {
        const current = step.id === activeStep;
        const complete = index < activeIndex;
        return (
          <li
            aria-current={current ? "step" : undefined}
            className={`flex items-center gap-2 border-b-2 px-2 py-3 text-xs ${
              current
                ? "border-foreground font-semibold text-foreground"
                : "border-transparent text-muted-foreground"
            }`}
            key={step.id}
          >
            <span
              className={`flex size-5 items-center justify-center rounded-full border text-3xs ${
                complete
                  ? "border-emerald-600 bg-emerald-600 text-white"
                  : current
                    ? "border-foreground"
                    : "border-border"
              }`}
            >
              {complete ? <CheckCircle2 className="size-3" /> : index + 1}
            </span>
            <span>{step.label}</span>
          </li>
        );
      })}
    </ol>
  );
}

export function NxtlinqPolicyReview({
  diffAcknowledged,
  explanation,
  initialization,
  isPreviewing,
  isRequestingRegeneration,
  onDiffAcknowledgedChange,
  onPolicyDraftChange,
  onRegenerate,
  onRegenerationGuidanceChange,
  onReset,
  originalPolicy,
  pending,
  policyDraft,
  policyDraftDirty,
  policyValidationError,
  preview,
  proposalSource,
  regenerationGuidance,
  regenerationRequested,
}: {
  diffAcknowledged: boolean;
  explanation: string;
  initialization: NxtlinqAttestInitializationStatus | undefined;
  isPreviewing: boolean;
  isRequestingRegeneration: boolean;
  onDiffAcknowledgedChange: (checked: boolean) => void;
  onPolicyDraftChange: (value: string) => void;
  onRegenerate: () => void;
  onRegenerationGuidanceChange: (value: string) => void;
  onReset: () => void;
  originalPolicy: NxtlinqPolicyDraft;
  pending: boolean;
  policyDraft: string;
  policyDraftDirty: boolean;
  policyValidationError: string | null;
  preview: NxtlinqManifestPreview | null;
  proposalSource: "agent" | "default";
  regenerationGuidance: string;
  regenerationRequested: boolean;
}) {
  return (
    <>
      <section className="space-y-2 rounded-xl border border-border/70 p-4">
        <h3 className="text-sm font-semibold">Why this access is needed</h3>
        <p className="whitespace-pre-wrap text-sm text-muted-foreground">
          {explanation}
        </p>
      </section>
      <section className="space-y-3">
        <div>
          <h3 className="text-sm font-semibold">
            Editable permission proposal
          </h3>
          <p className="text-xs text-muted-foreground">
            {proposalSource === "agent"
              ? "The Agent suggested this policy. "
              : "Buzz started with a conservative policy. "}
            Edit the requested capabilities, then review the resulting manifest
            diff. Buzz applies the locked safeguards below.
          </p>
        </div>
        <div className="rounded-lg border border-border/70 bg-muted/30 p-3">
          <p className="text-xs font-semibold">Locked safeguards</p>
          <ul className="mt-2 list-disc space-y-1 pl-4 text-xs text-muted-foreground">
            <li>Gateway-only audience and inert policy scope</li>
            <li>
              Sensitive file, credential, Git, key, and Nxtlinq metadata
              exclusions
            </li>
            <li>Buzz MCP session connection without tool invocation</li>
          </ul>
          <p className="mt-2 text-xs text-muted-foreground">
            These protections are added by Buzz and cannot be removed in this
            editor.
          </p>
        </div>
        <Textarea
          aria-label="Editable Nxtlinq permission proposal"
          className="min-h-64 resize-y font-mono text-xs leading-5"
          data-testid="nxtlinq-manifest-policy-editor"
          disabled={pending || regenerationRequested}
          onChange={(event) => onPolicyDraftChange(event.target.value)}
          value={policyDraft}
        />
        <div className="flex flex-wrap items-center justify-between gap-2">
          <p className="text-xs text-muted-foreground">
            {policyDraftDirty
              ? policyValidationError
                ? "Fix the policy error before reviewing the diff."
                : "Checking your edits and updating the diff…"
              : "The diff below reflects the latest valid policy."}
          </p>
          <Button
            disabled={
              pending ||
              regenerationRequested ||
              policyDraft === formatNxtlinqPolicyDraft(originalPolicy)
            }
            onClick={onReset}
            size="sm"
            type="button"
            variant="ghost"
          >
            {proposalSource === "agent"
              ? "Reset to Agent suggestion"
              : "Reset to safe baseline"}
          </Button>
        </div>
        <p className="text-xs text-muted-foreground">
          Reset only replaces this uncommitted editor draft. It does not change
          the project manifest.
        </p>
        {proposalSource === "agent" ? (
          <div className="space-y-3 rounded-lg border border-border/70 p-3">
            <div>
              <p className="text-xs font-semibold">Regenerate with Agent</p>
              <p className="text-xs text-muted-foreground">
                Ask the originating Agent to analyze the project again and
                submit a new proposal. Nothing is applied or signed.
              </p>
            </div>
            <Textarea
              aria-label="Optional guidance for regenerated Nxtlinq proposal"
              className="min-h-20 resize-y text-sm"
              disabled={
                pending || regenerationRequested || isRequestingRegeneration
              }
              onChange={(event) =>
                onRegenerationGuidanceChange(event.target.value)
              }
              placeholder="Optional: allow npm test, remove write access, limit reads to src/**…"
              value={regenerationGuidance}
            />
            <div className="flex items-center justify-between gap-3">
              <p className="text-xs text-muted-foreground">
                {regenerationRequested
                  ? "Request sent. Waiting for the Agent's new Desktop draft…"
                  : "The latest valid proposal and your guidance will be sent in the original channel."}
              </p>
              <Button
                disabled={
                  pending ||
                  policyDraftDirty ||
                  Boolean(policyValidationError) ||
                  regenerationRequested ||
                  isRequestingRegeneration
                }
                onClick={onRegenerate}
                size="sm"
                type="button"
                variant="outline"
              >
                {isRequestingRegeneration || regenerationRequested ? (
                  <LoaderCircle className="mr-2 size-4 animate-spin" />
                ) : (
                  <Sparkles className="mr-2 size-4" />
                )}
                {regenerationRequested
                  ? "Waiting for Agent"
                  : "Regenerate proposal"}
              </Button>
            </div>
          </div>
        ) : null}
        {policyValidationError ? (
          <p className="text-sm text-destructive" role="alert">
            {policyValidationError}
          </p>
        ) : null}
      </section>
      <section className="space-y-3">
        <div>
          <h3 className="text-sm font-semibold">
            Manifest diff: current vs proposed
          </h3>
          <p className="text-xs text-muted-foreground">
            Current manifest is shown on the left; your proposed manifest is
            shown on the right.
          </p>
        </div>
        {isPreviewing ? (
          <div className="flex items-center gap-2 text-sm text-muted-foreground">
            <LoaderCircle className="size-4 animate-spin" /> Reviewing manifest…
          </div>
        ) : initialization?.status !== "initialized" ? (
          <div className="rounded-lg border border-dashed border-border/70 p-3 text-sm text-muted-foreground">
            Initialize or repair Nxtlinq Attest before reviewing the exact
            manifest diff.
          </div>
        ) : preview?.changed ? (
          <div
            className="max-h-[360px] overflow-auto rounded-xl border border-border/60"
            data-testid="nxtlinq-manifest-diff"
          >
            <div className="sticky top-0 z-10 grid min-w-[780px] grid-cols-2 border-b border-border/60 bg-muted/95 text-xs font-medium backdrop-blur">
              <div className="border-r border-border/60 px-4 py-2.5">
                Current manifest
              </div>
              <div className="px-4 py-2.5">Proposed manifest</div>
            </div>
            <DiffViewer
              className="p-3"
              content={preview.unifiedDiff}
              fallbackFilePath="nxtlinq/agent.manifest.json"
              highlightChangedFragments
              viewType="split"
            />
          </div>
        ) : preview ? (
          <div className="flex items-center gap-2 rounded-lg bg-emerald-500/10 p-3 text-sm text-emerald-700 dark:text-emerald-400">
            <CheckCircle2 className="size-4" /> The proposed policy already
            matches the manifest.
          </div>
        ) : null}
        {preview && !isPreviewing && !policyDraftDirty ? (
          <div className="flex items-start gap-3 rounded-lg border border-border/70 p-3">
            <Checkbox
              checked={diffAcknowledged}
              disabled={
                pending ||
                regenerationRequested ||
                Boolean(policyValidationError)
              }
              id="nxtlinq-manifest-diff-reviewed"
              onCheckedChange={(checked) =>
                onDiffAcknowledgedChange(checked === true)
              }
            />
            <label
              className="text-sm leading-5"
              htmlFor="nxtlinq-manifest-diff-reviewed"
            >
              I reviewed the current and proposed manifest shown above.
            </label>
          </div>
        ) : null}
      </section>
    </>
  );
}

export function NxtlinqTrustAndActivation({
  activeStep,
  completed,
  configReady,
  isSavingConfig,
  onReceiptRootChange,
  onSaveOperatorConfig,
  onTrustStoreChange,
  operatorConfigSaved,
  receiptRoot,
  signResult,
  signatureRequired,
  trustStore,
}: {
  activeStep: NxtlinqSetupStep;
  completed: boolean;
  configReady: boolean;
  isSavingConfig: boolean;
  onReceiptRootChange: (path: string) => void;
  onSaveOperatorConfig: () => void;
  onTrustStoreChange: (path: string) => void;
  operatorConfigSaved: boolean;
  receiptRoot: string;
  signResult: NxtlinqManifestSignResult | null;
  signatureRequired: boolean;
  trustStore: string;
}) {
  if (activeStep === "trust") {
    return (
      <section
        className="space-y-3 rounded-xl border border-border/70 p-4"
        data-testid="nxtlinq-trust-step"
      >
        <div>
          <h3 className="text-sm font-semibold">Operator-owned trust</h3>
          <p className="text-xs text-muted-foreground">
            These paths remain local to Buzz and are never sent to the Agent.
          </p>
        </div>
        <div className="flex gap-2">
          <Input
            readOnly
            value={trustStore}
            placeholder="Select trusted-signers.json"
          />
          <Button
            onClick={() =>
              void pickNxtlinqTrustStore().then((path) => {
                if (path) onTrustStoreChange(path);
              })
            }
            type="button"
            variant="outline"
          >
            <FolderOpen className="mr-2 size-4" /> Trust store
          </Button>
        </div>
        <div className="flex gap-2">
          <Input
            readOnly
            value={receiptRoot}
            placeholder="Select receipt directory"
          />
          <Button
            onClick={() =>
              void pickNxtlinqDirectory().then((path) => {
                if (path) onReceiptRootChange(path);
              })
            }
            type="button"
            variant="outline"
          >
            <FolderOpen className="mr-2 size-4" /> Receipts
          </Button>
        </div>
        <Button
          disabled={!configReady || operatorConfigSaved || isSavingConfig}
          onClick={onSaveOperatorConfig}
          size="sm"
          type="button"
          variant="outline"
        >
          {operatorConfigSaved
            ? "Operator settings saved"
            : "Save operator settings"}
        </Button>
      </section>
    );
  }
  if (activeStep !== "activate") return null;
  return (
    <>
      {signatureRequired && !signResult ? (
        <div className="flex items-start gap-2 rounded-xl border border-amber-500/30 bg-amber-500/10 p-4 text-sm text-amber-800 dark:text-amber-300">
          <TriangleAlert className="mt-0.5 size-4 shrink-0" />
          <div className="space-y-2">
            <p>
              Buzz will sign with the owner key in secure storage. The key never
              enters the Agent or web interface.
            </p>
            <p className="text-xs">
              If the operating-system keyring was unavailable during
              initialization, Buzz uses its owner-only app-data fallback.
            </p>
          </div>
        </div>
      ) : null}
      {signResult ? (
        <div className="flex items-start gap-2 rounded-xl bg-emerald-500/10 p-4 text-sm text-emerald-700 dark:text-emerald-400">
          <CheckCircle2 className="mt-0.5 size-4 shrink-0" />
          <span>
            Manifest signed and trusted as {signResult.signerKeyId}. The private
            key was not shared with the Agent.
          </span>
        </div>
      ) : null}
      {completed ? (
        <div className="flex items-start gap-2 rounded-xl bg-emerald-500/10 p-4 text-sm text-emerald-700 dark:text-emerald-400">
          <CheckCircle2 className="mt-0.5 size-4 shrink-0" />
          <span>
            Gateway verified and saved. The Agent remains stopped; restart it
            when you are ready to use the new policy.
          </span>
        </div>
      ) : null}
    </>
  );
}

export function NxtlinqSetupFooter({
  activeStep,
  activeStepIndex,
  agentIsLocal,
  completed,
  configReady,
  diffAcknowledged,
  gatewayAvailable,
  gatewayLoading,
  initialization,
  isChecking,
  isInitializing,
  isInstalling,
  isInspectingInitialization,
  isSigning,
  onApplyManifest,
  onBack,
  onClose,
  onContinueFromTrust,
  onContinueToActivation,
  onContinueToPolicy,
  onEnable,
  onInitialize,
  onInstallGateway,
  onSign,
  operatorConfigSaved,
  pending,
  policyDraftDirty,
  policyValidationError,
  preview,
  projectRoot,
  regenerationRequested,
  signatureRequired,
  signResult,
  signerKeyId,
  workspaceMatches,
}: {
  activeStep: NxtlinqSetupStep;
  activeStepIndex: number;
  agentIsLocal: boolean;
  completed: boolean;
  configReady: boolean;
  diffAcknowledged: boolean;
  gatewayAvailable: boolean;
  gatewayLoading: boolean;
  initialization: NxtlinqAttestInitializationStatus | undefined;
  isChecking: boolean;
  isInitializing: boolean;
  isInstalling: boolean;
  isInspectingInitialization: boolean;
  isSigning: boolean;
  onApplyManifest: () => void;
  onBack: () => void;
  onClose: () => void;
  onContinueFromTrust: () => void;
  onContinueToActivation: () => void;
  onContinueToPolicy: () => void;
  onEnable: () => void;
  onInitialize: () => void;
  onInstallGateway: () => void;
  onSign: () => void;
  operatorConfigSaved: boolean;
  pending: boolean;
  policyDraftDirty: boolean;
  policyValidationError: string | null;
  preview: NxtlinqManifestPreview | null;
  projectRoot: string;
  regenerationRequested: boolean;
  signatureRequired: boolean;
  signResult: NxtlinqManifestSignResult | null;
  signerKeyId: string;
  workspaceMatches: boolean;
}) {
  let action: React.ReactNode;
  if (!projectRoot.trim()) {
    action = <Button disabled>Choose a project to continue</Button>;
  } else if (!workspaceMatches) {
    action = (
      <Button disabled>Use project as Agent workspace to continue</Button>
    );
  } else if (gatewayLoading) {
    action = (
      <Button disabled>
        <LoaderCircle className="mr-2 size-4 animate-spin" /> Checking tools…
      </Button>
    );
  } else if (!gatewayAvailable) {
    action = (
      <Button disabled={pending || gatewayLoading} onClick={onInstallGateway}>
        {isInstalling ? (
          <LoaderCircle className="mr-2 size-4 animate-spin" />
        ) : null}
        Install reviewed Gateway
      </Button>
    );
  } else if (isInspectingInitialization) {
    action = (
      <Button disabled>
        <LoaderCircle className="mr-2 size-4 animate-spin" /> Checking project…
      </Button>
    );
  } else if (initialization?.status === "missing") {
    action = (
      <Button
        disabled={pending || !agentIsLocal || !signerKeyId.trim()}
        onClick={onInitialize}
      >
        {isInitializing ? (
          <LoaderCircle className="mr-2 size-4 animate-spin" />
        ) : (
          <KeyRound className="mr-2 size-4" />
        )}
        Initialize securely
      </Button>
    );
  } else if (initialization && initialization.status !== "initialized") {
    action = (
      <Button disabled>Resolve Attest initialization to continue</Button>
    );
  } else if (activeStep === "workspace") {
    action = (
      <Button
        disabled={pending || initialization?.status !== "initialized"}
        onClick={onContinueToPolicy}
      >
        Continue to policy
      </Button>
    );
  } else if (activeStep === "trust") {
    action = (
      <Button disabled={pending || !configReady} onClick={onContinueFromTrust}>
        {operatorConfigSaved
          ? "Continue to policy"
          : "Save & continue to policy"}
      </Button>
    );
  } else if (activeStep === "policy" && preview?.changed) {
    action = (
      <Button
        disabled={
          pending ||
          policyDraftDirty ||
          regenerationRequested ||
          Boolean(policyValidationError) ||
          !diffAcknowledged
        }
        onClick={onApplyManifest}
      >
        Apply manifest changes
      </Button>
    );
  } else if (activeStep === "policy") {
    action = (
      <Button
        disabled={
          pending ||
          policyDraftDirty ||
          regenerationRequested ||
          Boolean(policyValidationError) ||
          !preview ||
          !diffAcknowledged
        }
        onClick={onContinueToActivation}
      >
        Continue to activation
      </Button>
    );
  } else if (signatureRequired && !signResult) {
    action = (
      <Button
        disabled={pending || !operatorConfigSaved || !workspaceMatches}
        onClick={onSign}
      >
        {isSigning ? (
          <LoaderCircle className="mr-2 size-4 animate-spin" />
        ) : null}
        Sign manifest securely
      </Button>
    );
  } else {
    action = (
      <Button
        disabled={
          pending ||
          !operatorConfigSaved ||
          !workspaceMatches ||
          initialization?.status !== "initialized" ||
          (signatureRequired && !signResult)
        }
        onClick={onEnable}
      >
        {isChecking ? (
          <LoaderCircle className="mr-2 size-4 animate-spin" />
        ) : null}
        Recheck &amp; enable Agent
      </Button>
    );
  }
  return (
    <div className="flex shrink-0 items-center justify-between gap-3 border-t border-border/60 px-6 py-4">
      <div className="flex items-center gap-2">
        <Button
          disabled={pending}
          onClick={onClose}
          type="button"
          variant="ghost"
        >
          {completed ? "Done" : "Cancel"}
        </Button>
        {!completed && activeStepIndex > 0 ? (
          <Button
            disabled={pending}
            onClick={onBack}
            type="button"
            variant="outline"
          >
            Back
          </Button>
        ) : null}
      </div>
      {!completed ? (
        <div className="flex items-center gap-2">{action}</div>
      ) : null}
    </div>
  );
}
