import {
  fromRawInstallRuntimeResult,
  type RawInstallRuntimeResult,
  type InstallRuntimeResult,
} from "./installTypes";
import { invokeTauri } from "./tauri";
import type { CommandAvailability } from "./types";

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
