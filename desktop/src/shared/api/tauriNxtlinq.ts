import {
  fromRawInstallRuntimeResult,
  type RawInstallRuntimeResult,
  type InstallRuntimeResult,
} from "./installTypes";
import { invokeTauri } from "./tauri";
import type { CommandAvailability } from "./types";
import type { NxtlinqPolicyDraft } from "@/features/agents/agentManagement";

type RawCommandAvailability = {
  command: string;
  resolved_path: string | null;
  available: boolean;
};

export type NxtlinqSetupCheckItem = {
  id: string;
  label: string;
  status:
    | "ready"
    | "found"
    | "valid"
    | "willCreate"
    | "missing"
    | "invalid"
    | "blocked";
  path: string | null;
  detail: string | null;
};

export type NxtlinqSetupCheckResult = {
  ready: boolean;
  checks: NxtlinqSetupCheckItem[];
  signerKeyId: string | null;
  gatewayExecutable: string | null;
  gatewayExecutableSha256: string | null;
  gatewayVersion: string | null;
  error: string | null;
};

export type NxtlinqAuthorizationConfig = {
  trustStore: string | null;
  receiptRoot: string;
};

export type NxtlinqManifestPreview = {
  manifestPath: string;
  currentManifest: string;
  proposedManifest: string;
  unifiedDiff: string;
  currentSha256: string;
  changed: boolean;
  requiresSignature: boolean;
};

export type NxtlinqManifestSignResult = {
  cancelled: boolean;
  signerKeyId: string | null;
  manifestSha256: string | null;
};

export type NxtlinqAttestInitializationState =
  | "missing"
  | "initialized"
  | "workspacePrivateKey"
  | "invalid";

export type NxtlinqAttestInitializationStatus = {
  status: NxtlinqAttestInitializationState;
  detail: string | null;
};

export type NxtlinqAttestInitializationResult = {
  cancelled: boolean;
  signerKeyId: string | null;
  publicKeyFingerprint: string | null;
  privateKeyStorage: string | null;
  trustStorePath: string | null;
};

export async function discoverNxtlinqAuthorizationGateway(): Promise<CommandAvailability> {
  const raw = await invokeTauri<RawCommandAvailability>(
    "discover_nxtlinq_authorization_gateway",
  );
  return {
    command: raw.command,
    resolvedPath: raw.resolved_path,
    available: raw.available,
  };
}

export async function installNxtlinqAuthorizationGateway(
  force: boolean,
): Promise<InstallRuntimeResult> {
  const raw = await invokeTauri<RawInstallRuntimeResult>(
    "install_nxtlinq_authorization_gateway",
    { force },
  );
  return fromRawInstallRuntimeResult(raw);
}

export async function uninstallNxtlinqAuthorizationGateway(): Promise<InstallRuntimeResult> {
  const raw = await invokeTauri<RawInstallRuntimeResult>(
    "uninstall_nxtlinq_authorization_gateway",
  );
  return fromRawInstallRuntimeResult(raw);
}

export async function getNxtlinqAuthorizationConfig(): Promise<NxtlinqAuthorizationConfig> {
  return invokeTauri<NxtlinqAuthorizationConfig>(
    "get_nxtlinq_authorization_config",
  );
}

export async function setNxtlinqAuthorizationConfig(
  config: NxtlinqAuthorizationConfig,
): Promise<NxtlinqAuthorizationConfig> {
  return invokeTauri<NxtlinqAuthorizationConfig>(
    "set_nxtlinq_authorization_config",
    { config },
  );
}

export async function pickNxtlinqTrustStore(): Promise<string | null> {
  return invokeTauri<string | null>("pick_nxtlinq_trust_store");
}

export async function pickNxtlinqDirectory(): Promise<string | null> {
  return invokeTauri<string | null>("pick_nxtlinq_directory");
}

export async function inspectNxtlinqAttestInitialization(input: {
  projectRoot: string;
}): Promise<NxtlinqAttestInitializationStatus> {
  return invokeTauri<NxtlinqAttestInitializationStatus>(
    "inspect_nxtlinq_attest_initialization",
    input,
  );
}

export async function initializeNxtlinqAttest(input: {
  agentPubkey: string;
  projectRoot: string;
  keyId: string;
}): Promise<NxtlinqAttestInitializationResult> {
  return invokeTauri<NxtlinqAttestInitializationResult>(
    "initialize_nxtlinq_attest",
    input,
  );
}

export async function checkNxtlinqAuthorizationSetup(input: {
  projectRoot: string;
  trustStore: string;
  receiptDirectory: string;
}): Promise<NxtlinqSetupCheckResult> {
  return invokeTauri<NxtlinqSetupCheckResult>(
    "check_nxtlinq_authorization_setup",
    {
      projectRoot: input.projectRoot,
      trustStore: input.trustStore,
      receiptDirectory: input.receiptDirectory,
    },
  );
}

export async function previewNxtlinqManifestPolicy(input: {
  projectRoot: string;
  policy: NxtlinqPolicyDraft;
}): Promise<NxtlinqManifestPreview> {
  return invokeTauri<NxtlinqManifestPreview>(
    "preview_nxtlinq_manifest_policy",
    input,
  );
}

export async function applyNxtlinqManifestPolicy(input: {
  projectRoot: string;
  policy: NxtlinqPolicyDraft;
  expectedSha256: string;
}): Promise<NxtlinqManifestPreview> {
  return invokeTauri<NxtlinqManifestPreview>(
    "apply_nxtlinq_manifest_policy",
    input,
  );
}

export async function signNxtlinqManifest(input: {
  agentPubkey: string;
  projectRoot: string;
  policy: NxtlinqPolicyDraft;
  expectedSha256: string;
  trustStore: string;
}): Promise<NxtlinqManifestSignResult> {
  return invokeTauri<NxtlinqManifestSignResult>("sign_nxtlinq_manifest", input);
}
