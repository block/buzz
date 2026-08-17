import * as React from "react";
import {
  CheckCircle2,
  FolderOpen,
  LoaderCircle,
  ShieldCheck,
} from "lucide-react";

import type {
  AgentManagementNxtlinqSetupRequest,
  NxtlinqPolicyDraft,
} from "../agentManagement";
import {
  useInstallNxtlinqAuthorizationGatewayMutation,
  useNxtlinqAuthorizationConfigQuery,
  useNxtlinqAuthorizationGatewayQuery,
  useSaveNxtlinqAuthorizationConfigMutation,
} from "../nxtlinqHooks";
import { useUpdateManagedAgentMutation } from "../hooks";
import {
  buildNxtlinqWrapperArgs,
  deriveNxtlinqReceiptDirectory,
} from "../lib/nxtlinqLaunchPreset";
import {
  applyNxtlinqManifestPolicy,
  checkNxtlinqAuthorizationSetup,
  initializeNxtlinqAttest,
  inspectNxtlinqAttestInitialization,
  pickNxtlinqDirectory,
  previewNxtlinqManifestPolicy,
  signNxtlinqManifest,
  type NxtlinqManifestPreview,
  type NxtlinqManifestSignResult,
} from "@/shared/api/tauriNxtlinq";
import type { ManagedAgent } from "@/shared/api/types";
import { sendChannelMessage } from "@/shared/api/tauri";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { Input } from "@/shared/ui/input";
import { NxtlinqInitializationSuccess } from "./NxtlinqInitializationSuccess";
import {
  NXTLINQ_SETUP_STEPS,
  NxtlinqPolicyReview,
  NxtlinqSetupFooter,
  NxtlinqSetupProgress,
  type NxtlinqSetupStep,
  NxtlinqTrustAndActivation,
} from "./NxtlinqSetupReviewControls";
import {
  formatNxtlinqPolicyDraft,
  parseEditableNxtlinqPolicyDraft,
  policyFromNxtlinqManifestJson,
} from "./nxtlinqPolicyDraft";

type Props = {
  agent: ManagedAgent | undefined;
  onOpenChange: (open: boolean) => void;
  proposalSource?: "agent" | "default";
  request: AgentManagementNxtlinqSetupRequest;
};

export function NxtlinqSetupReviewDialog({
  agent,
  onOpenChange,
  proposalSource = "agent",
  request,
}: Props) {
  const gatewayQuery = useNxtlinqAuthorizationGatewayQuery();
  const configQuery = useNxtlinqAuthorizationConfigQuery();
  const installMutation = useInstallNxtlinqAuthorizationGatewayMutation();
  const saveConfigMutation = useSaveNxtlinqAuthorizationConfigMutation();
  const updateAgentMutation = useUpdateManagedAgentMutation();
  const [activeStep, setActiveStep] =
    React.useState<NxtlinqSetupStep>("workspace");
  const [preview, setPreview] = React.useState<NxtlinqManifestPreview | null>(
    null,
  );
  const [reviewedPolicy, setReviewedPolicy] =
    React.useState<NxtlinqPolicyDraft>(request.request.policy);
  const [policyDraft, setPolicyDraft] = React.useState(() =>
    formatNxtlinqPolicyDraft(request.request.policy),
  );
  const [policyDraftDirty, setPolicyDraftDirty] = React.useState(false);
  const [policyValidationError, setPolicyValidationError] = React.useState<
    string | null
  >(null);
  const [diffAcknowledged, setDiffAcknowledged] = React.useState(false);
  const [regenerationGuidance, setRegenerationGuidance] = React.useState("");
  const [isRequestingRegeneration, setIsRequestingRegeneration] =
    React.useState(false);
  const [regenerationRequested, setRegenerationRequested] =
    React.useState(false);
  const [projectRoot, setProjectRoot] = React.useState(
    request.request.projectRoot,
  );
  const [adoptedWorkspace, setAdoptedWorkspace] = React.useState<string | null>(
    null,
  );
  const [trustStore, setTrustStore] = React.useState("");
  const [receiptRoot, setReceiptRoot] = React.useState("");
  const [isInspectingInitialization, setIsInspectingInitialization] =
    React.useState(true);
  const [initialization, setInitialization] = React.useState<
    Awaited<ReturnType<typeof inspectNxtlinqAttestInitialization>> | undefined
  >();
  const [isPreviewing, setIsPreviewing] = React.useState(true);
  const [isInitializing, setIsInitializing] = React.useState(false);
  const [initializationRevision, setInitializationRevision] = React.useState(0);
  const [signerKeyId, setSignerKeyId] = React.useState(() =>
    `${request.request.policy.name}-owner`
      .replace(/[^A-Za-z0-9._:/-]+/g, "-")
      .slice(0, 128),
  );
  const [initializedSigner, setInitializedSigner] = React.useState<{
    signerKeyId: string | null;
    publicKeyFingerprint: string | null;
    privateKeyStorage: string | null;
    trustStorePath: string | null;
  } | null>(null);
  const [isApplying, setIsApplying] = React.useState(false);
  const [isSigning, setIsSigning] = React.useState(false);
  const [isChecking, setIsChecking] = React.useState(false);
  const [signatureRequired, setSignatureRequired] = React.useState(false);
  const [signResult, setSignResult] =
    React.useState<NxtlinqManifestSignResult | null>(null);
  const [completed, setCompleted] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const adoptedCurrentPolicyFor = React.useRef<string | null>(null);
  const previousRequestId = React.useRef(request.requestId);

  React.useEffect(() => {
    if (!configQuery.data) return;
    setTrustStore(configQuery.data.trustStore ?? "");
    setReceiptRoot(configQuery.data.receiptRoot);
  }, [configQuery.data]);

  React.useEffect(() => {
    // Initialization increments this nonce so inspection and preview rerun
    // without closing the owner review draft.
    void initializationRevision;
    let cancelled = false;
    setIsInspectingInitialization(true);
    setIsPreviewing(true);
    setDiffAcknowledged(false);
    setError(null);
    setPreview(null);
    setInitialization(undefined);
    setSignatureRequired(false);
    setSignResult(null);
    setCompleted(false);
    if (!projectRoot.trim()) {
      setIsInspectingInitialization(false);
      setIsPreviewing(false);
      return () => {
        cancelled = true;
      };
    }
    void (async () => {
      try {
        const inspected = await inspectNxtlinqAttestInitialization({
          projectRoot,
        });
        if (cancelled) return;
        setInitialization(inspected);
        setIsInspectingInitialization(false);
        if (inspected.status !== "initialized") {
          setIsPreviewing(false);
          return;
        }
        const value = await previewNxtlinqManifestPolicy({
          projectRoot,
          policy: reviewedPolicy,
        });
        if (cancelled) return;
        if (
          proposalSource === "default" &&
          adoptedCurrentPolicyFor.current !== projectRoot
        ) {
          adoptedCurrentPolicyFor.current = projectRoot;
          const currentPolicy = policyFromNxtlinqManifestJson(
            value.currentManifest,
          );
          if (currentPolicy) {
            setReviewedPolicy(currentPolicy);
            setPolicyDraft(formatNxtlinqPolicyDraft(currentPolicy));
            setPolicyDraftDirty(false);
            return;
          }
        }
        setPreview(value);
      } catch (cause) {
        if (!cancelled) {
          setError(
            cause instanceof Error
              ? cause.message
              : "Could not inspect the Nxtlinq project.",
          );
        }
      } finally {
        if (!cancelled) {
          setIsInspectingInitialization(false);
          setIsPreviewing(false);
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [initializationRevision, projectRoot, proposalSource, reviewedPolicy]);

  React.useEffect(() => {
    setReviewedPolicy(request.request.policy);
    setPolicyDraft(formatNxtlinqPolicyDraft(request.request.policy));
    setPolicyDraftDirty(false);
    setPolicyValidationError(null);
    setDiffAcknowledged(false);
    setSignerKeyId(
      `${request.request.policy.name}-owner`
        .replace(/[^A-Za-z0-9._:/-]+/g, "-")
        .slice(0, 128),
    );
  }, [request.request.policy]);

  React.useEffect(() => {
    if (previousRequestId.current === request.requestId) return;
    previousRequestId.current = request.requestId;
    setRegenerationRequested(false);
    setRegenerationGuidance("");
    setDiffAcknowledged(false);
    setSignResult(null);
    setCompleted(false);
    setError(null);
    if (request.request.projectRoot === projectRoot) {
      setActiveStep("policy");
      return;
    }
    setActiveStep("workspace");
    setAdoptedWorkspace(null);
    setInitializedSigner(null);
    setProjectRoot(request.request.projectRoot);
  }, [projectRoot, request.request.projectRoot, request.requestId]);

  React.useEffect(() => {
    if (!policyDraftDirty) return;
    const timeout = window.setTimeout(() => {
      let parsed: unknown;
      try {
        parsed = JSON.parse(policyDraft);
      } catch (cause) {
        setPolicyValidationError(
          `Manifest policy is not valid JSON: ${
            cause instanceof Error ? cause.message : "unknown parse error"
          }`,
        );
        return;
      }
      const policy = parseEditableNxtlinqPolicyDraft(parsed);
      if (!policy) {
        setPolicyValidationError(
          "The editable policy is not valid. Use relative filesystem paths and explicit supported capabilities; terminal access must include PATH.",
        );
        return;
      }
      setPolicyValidationError(null);
      setPolicyDraftDirty(false);
      setReviewedPolicy(policy);
    }, 450);
    return () => window.clearTimeout(timeout);
  }, [policyDraft, policyDraftDirty]);

  const configReady =
    trustStore.trim().length > 0 && receiptRoot.trim().length > 0;
  const operatorConfigSaved =
    configReady &&
    configQuery.data?.trustStore === trustStore.trim() &&
    configQuery.data.receiptRoot === receiptRoot.trim();
  const setupSteps = React.useMemo(
    () =>
      NXTLINQ_SETUP_STEPS.filter(
        (step) => step.id !== "trust" || !operatorConfigSaved,
      ),
    [operatorConfigSaved],
  );
  const activeStepIndex = setupSteps.findIndex(
    (step) => step.id === activeStep,
  );
  const receiptDirectory = agent
    ? deriveNxtlinqReceiptDirectory(receiptRoot, agent.pubkey)
    : "";
  const workspaceMatches = Boolean(
    agent &&
      (agent.workingDirectory === projectRoot ||
        adoptedWorkspace === projectRoot),
  );
  const pending =
    installMutation.isPending ||
    saveConfigMutation.isPending ||
    updateAgentMutation.isPending ||
    isInspectingInitialization ||
    isInitializing ||
    isApplying ||
    isSigning ||
    isChecking;

  async function saveOperatorConfig(): Promise<boolean> {
    setError(null);
    try {
      await saveConfigMutation.mutateAsync({
        trustStore: trustStore.trim() || null,
        receiptRoot: receiptRoot.trim(),
      });
      return true;
    } catch (cause) {
      setError(
        cause instanceof Error
          ? cause.message
          : "Could not save Nxtlinq operator settings.",
      );
      return false;
    }
  }

  async function installGateway() {
    setError(null);
    try {
      const result = await installMutation.mutateAsync(false);
      if (!result.success) {
        throw new Error(
          result.steps.at(-1)?.hint ?? "Nxtlinq Gateway installation failed.",
        );
      }
      await gatewayQuery.refetch();
    } catch (cause) {
      setError(
        cause instanceof Error ? cause.message : "Gateway installation failed.",
      );
    }
  }

  async function adoptProjectAsAgentWorkspace() {
    if (!agent) {
      setError("The requesting managed Agent is no longer available.");
      return;
    }
    if (agent.backend.type !== "local") {
      setError(
        "Changing the reviewed project currently requires a local managed Agent.",
      );
      return;
    }
    setError(null);
    try {
      await updateAgentMutation.mutateAsync({
        pubkey: agent.pubkey,
        workingDirectory: projectRoot,
      });
      setAdoptedWorkspace(projectRoot);
    } catch (cause) {
      setError(
        cause instanceof Error
          ? cause.message
          : "Could not set the Agent workspace.",
      );
    }
  }

  async function chooseProjectRoot() {
    setError(null);
    try {
      const selected = await pickNxtlinqDirectory();
      if (selected && selected !== projectRoot) {
        setActiveStep("workspace");
        setAdoptedWorkspace(null);
        setInitializedSigner(null);
        setProjectRoot(selected);
      }
    } catch (cause) {
      setError(
        cause instanceof Error
          ? cause.message
          : "Could not select the project workspace.",
      );
    }
  }

  async function initializeProject() {
    if (initialization?.status !== "missing") {
      setError(
        "This project is no longer ready for Nxtlinq Attest initialization.",
      );
      return;
    }
    if (!agent) {
      setError("The requesting managed Agent is no longer available.");
      return;
    }
    if (!signerKeyId.trim()) {
      setError("Enter a signer key ID before initializing the project.");
      return;
    }
    setIsInitializing(true);
    setError(null);
    try {
      const result = await initializeNxtlinqAttest({
        agentPubkey: agent.pubkey,
        projectRoot,
        keyId: signerKeyId.trim(),
      });
      if (!result.cancelled) {
        setInitializedSigner({
          signerKeyId: result.signerKeyId ?? null,
          publicKeyFingerprint: result.publicKeyFingerprint ?? null,
          privateKeyStorage: result.privateKeyStorage ?? null,
          trustStorePath: result.trustStorePath ?? null,
        });
        if (result.trustStorePath) {
          setTrustStore(result.trustStorePath);
          await configQuery.refetch();
        }
        setInitializationRevision((revision) => revision + 1);
        setActiveStep("policy");
      }
    } catch (cause) {
      setError(
        cause instanceof Error
          ? cause.message
          : "Could not initialize Nxtlinq Attest for this project.",
      );
    } finally {
      setIsInitializing(false);
    }
  }

  function resetPolicyDraft() {
    const policy = request.request.policy;
    setError(null);
    setPolicyValidationError(null);
    setDiffAcknowledged(false);
    setPolicyDraft(formatNxtlinqPolicyDraft(policy));
    setPolicyDraftDirty(false);
    setReviewedPolicy(policy);
  }

  async function requestAgentRegeneration() {
    if (!agent || proposalSource !== "agent") {
      setError(
        "Regeneration requires a setup draft opened from an Agent conversation.",
      );
      return;
    }
    setIsRequestingRegeneration(true);
    setError(null);
    setDiffAcknowledged(false);
    try {
      const guidance = regenerationGuidance.trim();
      await sendChannelMessage(
        request.request.channelId,
        [
          `@${agent.name} Regenerate the Nxtlinq authorization setup draft for ${projectRoot}.`,
          "Submit a new structured Desktop review draft now by invoking the trusted nxtlinq_setup tool in this same turn.",
          "The current owner-reviewed proposal and owner guidance below are sufficient evidence. Project-file inspection is optional and must not delay submission.",
          "If any file, MCP, shell, or help lookup is denied, do not retry, do not run buzz agents nxtlinq-setup or --help through shell, and do not stop. Continue directly with the structured tool using the smallest useful proposal.",
          "Do not read secrets, signing material, .env files, or nxtlinq/**. Do not install, sign, or modify project files.",
          `Use this current owner-reviewed proposal as context:\n${JSON.stringify(reviewedPolicy, null, 2)}`,
          guidance ? `Owner revision guidance:\n${guidance}` : "",
        ]
          .filter(Boolean)
          .join("\n\n"),
        null,
        undefined,
        [agent.pubkey],
      );
      setRegenerationRequested(true);
    } catch (cause) {
      setError(
        cause instanceof Error
          ? cause.message
          : "Could not ask the Agent to regenerate the policy.",
      );
    } finally {
      setIsRequestingRegeneration(false);
    }
  }

  async function applyManifest() {
    if (!preview) return;
    setIsApplying(true);
    setError(null);
    try {
      const refreshed = await applyNxtlinqManifestPolicy({
        projectRoot,
        policy: reviewedPolicy,
        expectedSha256: preview.currentSha256,
      });
      setPreview(refreshed);
      setSignatureRequired(true);
      setActiveStep("activate");
    } catch (cause) {
      setError(
        cause instanceof Error
          ? cause.message
          : "Could not apply the Nxtlinq manifest.",
      );
    } finally {
      setIsApplying(false);
    }
  }

  async function signManifest() {
    if (!agent) {
      setError("The requesting managed Agent is no longer available.");
      return;
    }
    if (agent.backend.type !== "local") {
      setError(
        "One-click signing currently requires a local managed Agent. Stop remote Agents at their provider before signing.",
      );
      return;
    }
    if (!operatorConfigSaved || !trustStore.trim()) {
      setError("Save the operator trust settings before signing.");
      return;
    }
    if (!workspaceMatches) {
      setError(
        "Use the reviewed project as the Agent workspace before signing.",
      );
      return;
    }
    setIsSigning(true);
    setError(null);
    try {
      // Recover when the initial dialog preview was temporarily unavailable.
      // A successful apply already returns the exact preview/hash that must be
      // signed, so preserve it; the native signer independently enforces that
      // expected hash and rejects any review-to-sign file change.
      const signingPreview =
        preview ??
        (await previewNxtlinqManifestPolicy({
          projectRoot,
          policy: reviewedPolicy,
        }));
      if (!preview) setPreview(signingPreview);
      if (signingPreview.changed) {
        throw new Error(
          "The manifest still differs from the reviewed policy. Apply the manifest changes before signing.",
        );
      }
      const result = await signNxtlinqManifest({
        agentPubkey: agent.pubkey,
        projectRoot,
        policy: reviewedPolicy,
        expectedSha256: signingPreview.currentSha256,
        trustStore: trustStore.trim(),
      });
      if (!result.cancelled) {
        setSignResult(result);
      }
    } catch (cause) {
      setError(
        cause instanceof Error
          ? cause.message
          : "Could not sign and verify the Nxtlinq manifest.",
      );
    } finally {
      setIsSigning(false);
    }
  }

  async function recheckAndEnable() {
    if (!agent) {
      setError("The requesting managed Agent is no longer available.");
      return;
    }
    if (!workspaceMatches) {
      setError(
        "Use the reviewed project as the Agent workspace before enabling authorization.",
      );
      return;
    }
    setIsChecking(true);
    setError(null);
    try {
      const result = await checkNxtlinqAuthorizationSetup({
        projectRoot,
        trustStore: trustStore.trim(),
        receiptDirectory,
      });
      if (
        !result.ready ||
        !result.gatewayExecutable ||
        !result.gatewayExecutableSha256
      ) {
        const failed = result.checks.find((check) =>
          ["missing", "invalid", "blocked"].includes(check.status),
        );
        if (failed?.id === "manifest") {
          setSignatureRequired(true);
        }
        throw new Error(
          failed?.detail ??
            result.error ??
            "Sign the manifest with the owner-controlled private key, then recheck.",
        );
      }
      const envVars: Record<string, string> = {
        ...agent.envVars,
        BUZZ_AGENT_NXTLINQ_PERMISSION_BRIDGE: "1",
        BUZZ_AGENT_REQUIRE_REPLY: agent.envVars.BUZZ_AGENT_REQUIRE_REPLY || "1",
      };
      delete envVars.BUZZ_ACP_TRUST_NXTLINQ_GATEWAY;
      await updateAgentMutation.mutateAsync({
        pubkey: agent.pubkey,
        workingDirectory: projectRoot,
        commandWrapper: {
          command: result.gatewayExecutable,
          args: buildNxtlinqWrapperArgs({
            project: projectRoot,
            trustStore: trustStore.trim(),
            receiptDirectory,
            passEnvironment: Object.keys(envVars),
          }),
          authorization: {
            kind: "nxtlinq_gateway",
            executable: result.gatewayExecutable,
            sha256: result.gatewayExecutableSha256,
          },
        },
        envVars,
      });
      setCompleted(true);
    } catch (cause) {
      setError(
        cause instanceof Error
          ? cause.message
          : "Could not verify and enable Nxtlinq authorization.",
      );
    } finally {
      setIsChecking(false);
    }
  }

  function goToPreviousStep() {
    const previous = setupSteps[activeStepIndex - 1];
    if (previous) {
      setError(null);
      setActiveStep(previous.id);
    }
  }

  async function continueFromTrust() {
    if (!operatorConfigSaved && !(await saveOperatorConfig())) return;
    setError(null);
    setActiveStep("policy");
  }

  return (
    <Dialog onOpenChange={onOpenChange} open>
      <DialogContent
        className="flex max-h-[90vh] max-w-6xl flex-col overflow-hidden p-0"
        data-testid="nxtlinq-setup-review-dialog"
      >
        <DialogHeader className="shrink-0 border-b border-border/60 px-6 py-5 pr-14">
          <DialogTitle className="flex items-center gap-2">
            <ShieldCheck className="size-5" />
            Review Nxtlinq authorization setup
          </DialogTitle>
          <DialogDescription>
            {projectRoot ? (
              <>
                Review the{" "}
                {proposalSource === "agent" ? "suggested" : "default"} policy
                for <span className="font-mono">{projectRoot}</span>. Nothing
                changes until you approve.
              </>
            ) : (
              "Choose the project this Agent should use. Nothing changes until you approve."
            )}
          </DialogDescription>
        </DialogHeader>

        <NxtlinqSetupProgress activeStep={activeStep} steps={setupSteps} />

        <div className="min-h-0 flex-1 space-y-5 overflow-y-auto px-6 py-5">
          {activeStep === "workspace" ? (
            <section
              className="space-y-3 rounded-xl border border-border/70 p-4"
              data-testid="nxtlinq-workspace-step"
            >
              <div>
                <h3 className="text-sm font-semibold">
                  Target project workspace
                </h3>
                <p className="text-xs text-muted-foreground">
                  Confirm the project. Buzz will initialize Attest if needed.
                </p>
              </div>
              <div className="flex gap-2">
                <Input readOnly value={projectRoot} />
                <Button
                  disabled={pending}
                  onClick={() => void chooseProjectRoot()}
                  type="button"
                  variant="outline"
                >
                  <FolderOpen className="mr-2 size-4" /> Browse
                </Button>
              </div>
              {workspaceMatches ? (
                <div className="flex items-center gap-2 text-xs text-emerald-700 dark:text-emerald-400">
                  <CheckCircle2 className="size-4" /> This is the Agent
                  workspace.
                </div>
              ) : (
                <div className="flex items-center justify-between gap-3 rounded-lg bg-amber-500/10 p-3">
                  <p className="text-xs text-amber-800 dark:text-amber-300">
                    Initialization, signing, and enablement are locked until you
                    explicitly use this project as the local Agent workspace.
                  </p>
                  <Button
                    disabled={pending || !agent || !projectRoot.trim()}
                    onClick={() => void adoptProjectAsAgentWorkspace()}
                    size="sm"
                    type="button"
                    variant="outline"
                  >
                    Use as Agent workspace
                  </Button>
                </div>
              )}
            </section>
          ) : null}
          {activeStep === "workspace" && gatewayQuery.isLoading ? (
            <section className="flex items-center gap-2 rounded-xl border border-border/70 p-4 text-sm text-muted-foreground">
              <LoaderCircle className="size-4 animate-spin" /> Checking reviewed
              Nxtlinq tools…
            </section>
          ) : activeStep === "workspace" && !gatewayQuery.data?.available ? (
            <section className="space-y-3 rounded-xl border border-border/70 p-4">
              <div>
                <h3 className="text-sm font-semibold">
                  Install reviewed Nxtlinq tools
                </h3>
                <p className="text-xs text-muted-foreground">
                  Buzz needs its pinned Gateway and Attest tools before it can
                  initialize this project. Installation stays inside
                  Buzz-managed application storage.
                </p>
              </div>
            </section>
          ) : activeStep === "workspace" && isInspectingInitialization ? (
            <section className="flex items-center gap-2 rounded-xl border border-border/70 p-4 text-sm text-muted-foreground">
              <LoaderCircle className="size-4 animate-spin" /> Checking Nxtlinq
              Attest initialization…
            </section>
          ) : activeStep === "workspace" &&
            initialization?.status === "missing" ? (
            <section
              className="space-y-4 rounded-xl border border-amber-500/30 bg-amber-500/5 p-4"
              data-testid="nxtlinq-project-initialization"
            >
              <div>
                <h3 className="text-sm font-semibold">
                  Initialize Nxtlinq Attest
                </h3>
                <p className="text-xs text-muted-foreground">
                  Buzz will initialize Attest, protect the private key in secure
                  storage, and keep only public files in the project.
                </p>
              </div>
              <label
                className="block space-y-1.5 text-xs font-medium"
                htmlFor="nxtlinq-signer-key-id"
              >
                Signer key ID
                <Input
                  aria-describedby="nxtlinq-signer-key-id-help"
                  disabled={pending}
                  id="nxtlinq-signer-key-id"
                  onChange={(event) => setSignerKeyId(event.target.value)}
                  placeholder="my-project-policy-2026"
                  value={signerKeyId}
                />
                <span
                  className="block text-xs font-normal text-muted-foreground"
                  id="nxtlinq-signer-key-id-help"
                >
                  Public label for this signing identity, such as
                  my-project-owner. Do not enter a key or file path.
                </span>
              </label>
              <div className="rounded-lg border border-border/70 bg-background p-3 text-xs text-muted-foreground">
                <p>
                  <strong className="font-semibold text-foreground">
                    Initialize secure signing identity
                  </strong>{" "}
                  stops the Agent, initializes Attest in protected staging, and
                  stores the owner key in the system keychain. If the keychain
                  is unavailable, Buzz uses owner-only app storage. The Agent
                  receives only public project files.
                </p>
                {!workspaceMatches ? (
                  <p className="mt-2 font-medium text-amber-800 dark:text-amber-300">
                    Use this project as the Agent workspace before initializing.
                  </p>
                ) : null}
              </div>
            </section>
          ) : activeStep === "workspace" &&
            initialization?.status === "workspacePrivateKey" ? (
            <section
              className="space-y-2 rounded-xl border border-destructive/40 bg-destructive/5 p-4"
              data-testid="nxtlinq-project-initialization-blocked"
            >
              <h3 className="text-sm font-semibold text-destructive">
                Private key found in the Agent workspace
              </h3>
              <p className="text-xs text-muted-foreground">
                Buzz will not continue while signing material is reachable by
                the Agent. Move the private key to an owner-controlled location
                outside this project, repair the public-key-only initialization,
                then reopen this review.
              </p>
              {initialization.detail ? (
                <p className="text-xs text-destructive">
                  {initialization.detail}
                </p>
              ) : null}
            </section>
          ) : activeStep === "workspace" &&
            initialization?.status === "invalid" ? (
            <section
              className="space-y-2 rounded-xl border border-destructive/40 bg-destructive/5 p-4"
              data-testid="nxtlinq-project-initialization-blocked"
            >
              <h3 className="text-sm font-semibold text-destructive">
                Nxtlinq Attest initialization needs attention
              </h3>
              <p className="text-xs text-muted-foreground">
                Buzz found existing Nxtlinq state but could not validate it. No
                manifest changes, signing, or authorization enablement are
                available until the project is repaired.
              </p>
              {initialization.detail ? (
                <p className="text-xs text-destructive">
                  {initialization.detail}
                </p>
              ) : null}
            </section>
          ) : activeStep === "workspace" &&
            initialization?.status === "initialized" ? (
            <section className="flex items-start gap-2 rounded-xl bg-emerald-500/10 p-4 text-sm text-emerald-700 dark:text-emerald-400">
              <CheckCircle2 className="mt-0.5 size-4 shrink-0" />
              <div>
                <p className="font-medium">Nxtlinq Attest is initialized.</p>
                <p className="mt-1 text-xs">
                  The project contains validated public identity and manifest
                  state. Continue to review this Buzz installation&apos;s local
                  trust paths.
                </p>
              </div>
            </section>
          ) : null}
          {(activeStep === "trust" || activeStep === "policy") &&
          initializedSigner ? (
            <NxtlinqInitializationSuccess {...initializedSigner} />
          ) : null}
          {activeStep === "policy" ? (
            <NxtlinqPolicyReview
              diffAcknowledged={diffAcknowledged}
              explanation={request.request.explanation}
              initialization={initialization}
              isPreviewing={isPreviewing}
              isRequestingRegeneration={isRequestingRegeneration}
              onDiffAcknowledgedChange={setDiffAcknowledged}
              onPolicyDraftChange={(value) => {
                setPolicyDraft(value);
                setPolicyDraftDirty(true);
                setPolicyValidationError(null);
                setDiffAcknowledged(false);
                setSignResult(null);
                setCompleted(false);
              }}
              onRegenerate={() => void requestAgentRegeneration()}
              onRegenerationGuidanceChange={setRegenerationGuidance}
              onReset={resetPolicyDraft}
              originalPolicy={request.request.policy}
              pending={pending}
              policyDraft={policyDraft}
              policyDraftDirty={policyDraftDirty}
              policyValidationError={policyValidationError}
              preview={preview}
              proposalSource={proposalSource}
              regenerationGuidance={regenerationGuidance}
              regenerationRequested={regenerationRequested}
            />
          ) : null}

          <NxtlinqTrustAndActivation
            activeStep={activeStep}
            completed={completed}
            configReady={configReady}
            isSavingConfig={saveConfigMutation.isPending}
            onReceiptRootChange={setReceiptRoot}
            onSaveOperatorConfig={() => void saveOperatorConfig()}
            onTrustStoreChange={setTrustStore}
            operatorConfigSaved={operatorConfigSaved}
            receiptRoot={receiptRoot}
            signResult={signResult}
            signatureRequired={signatureRequired}
            trustStore={trustStore}
          />
          {error ? (
            <p className="text-sm text-destructive" role="alert">
              {error}
            </p>
          ) : null}
        </div>

        <NxtlinqSetupFooter
          activeStep={activeStep}
          activeStepIndex={activeStepIndex}
          agentIsLocal={agent?.backend.type === "local"}
          completed={completed}
          configReady={configReady}
          diffAcknowledged={diffAcknowledged}
          gatewayAvailable={Boolean(gatewayQuery.data?.available)}
          gatewayLoading={gatewayQuery.isLoading}
          initialization={initialization}
          isChecking={isChecking}
          isInitializing={isInitializing}
          isInstalling={installMutation.isPending}
          isInspectingInitialization={isInspectingInitialization}
          isSigning={isSigning}
          onApplyManifest={() => void applyManifest()}
          onBack={goToPreviousStep}
          onClose={() => onOpenChange(false)}
          onContinueFromTrust={() => void continueFromTrust()}
          onContinueToActivation={() => {
            setError(null);
            setActiveStep("activate");
          }}
          onContinueToPolicy={() => {
            setError(null);
            setActiveStep(operatorConfigSaved ? "policy" : "trust");
          }}
          onEnable={() => void recheckAndEnable()}
          onInitialize={() => void initializeProject()}
          onInstallGateway={() => void installGateway()}
          onSign={() => void signManifest()}
          operatorConfigSaved={operatorConfigSaved}
          pending={pending}
          policyDraftDirty={policyDraftDirty}
          policyValidationError={policyValidationError}
          preview={preview}
          projectRoot={projectRoot}
          regenerationRequested={regenerationRequested}
          signatureRequired={signatureRequired}
          signResult={signResult}
          signerKeyId={signerKeyId}
          workspaceMatches={workspaceMatches}
        />
      </DialogContent>
    </Dialog>
  );
}
