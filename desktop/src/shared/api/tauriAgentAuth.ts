import type {
  AcpAuthMethod,
  AcpAuthMethodsResult,
  ConnectAcpRuntimeResult,
} from "@/shared/api/types";
import { invokeTauri } from "@/shared/api/tauri";

type RawAcpAuthMethod = {
  id: string;
  name: string;
  description?: string | null;
  type?: string | null;
  args?: string[];
  command?: string[];
  _meta?: unknown;
};

export type RawAcpAuthMethodsResult = {
  methods: RawAcpAuthMethod[];
};

export type RawConnectAcpRuntimeResult = {
  launched: boolean;
};

function fromRawAcpAuthMethod(method: RawAcpAuthMethod): AcpAuthMethod {
  return {
    id: method.id,
    name: method.name,
    description: method.description ?? null,
    type: method.type ?? null,
    args: method.args ?? [],
    command: method.command ?? [],
    meta: method._meta ?? null,
  };
}

export async function discoverAcpAuthMethods(
  runtimeId: string,
): Promise<AcpAuthMethodsResult> {
  const raw = await invokeTauri<RawAcpAuthMethodsResult>(
    "discover_acp_auth_methods",
    { runtimeId },
  );
  return { methods: raw.methods.map(fromRawAcpAuthMethod) };
}

export async function connectAcpRuntime(
  runtimeId: string,
  methodId: string,
): Promise<ConnectAcpRuntimeResult> {
  return invokeTauri<RawConnectAcpRuntimeResult>("connect_acp_runtime", {
    request: { runtimeId, methodId },
  });
}
