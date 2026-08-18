import {
  parseNxtlinqPolicyDraft,
  REQUIRED_NXTLINQ_SENSITIVE_EXCLUDES,
  type NxtlinqPolicyDraft,
} from "../agentManagement";

export function formatNxtlinqPolicyDraft(policy: NxtlinqPolicyDraft) {
  const capabilities: unknown[] = [];
  for (const capability of policy.capabilities) {
    if (
      capability.type === "filesystem:read" ||
      capability.type === "filesystem:write"
    ) {
      const customExcludes = (capability.exclude ?? []).filter(
        (pattern) =>
          !REQUIRED_NXTLINQ_SENSITIVE_EXCLUDES.includes(
            pattern as (typeof REQUIRED_NXTLINQ_SENSITIVE_EXCLUDES)[number],
          ),
      );
      const { exclude: _exclude, ...rest } = capability;
      capabilities.push({
        ...rest,
        ...(customExcludes.length > 0 ? { exclude: customExcludes } : {}),
      });
      continue;
    }
    if (capability.type === "mcp:connect") {
      const servers = capability.servers.filter(
        (server) => server !== "buzz-dev-mcp",
      );
      if (servers.length > 0) capabilities.push({ ...capability, servers });
      continue;
    }
    capabilities.push(capability);
  }
  return JSON.stringify(
    {
      name: policy.name,
      version: policy.version,
      capabilities,
      ...(policy.exp === undefined ? {} : { exp: policy.exp }),
    },
    null,
    2,
  );
}

export function parseEditableNxtlinqPolicyDraft(
  value: unknown,
): NxtlinqPolicyDraft | null {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return null;
  }
  const editable = value as Record<string, unknown>;
  if (
    Object.keys(editable).some(
      (key) => !["name", "version", "capabilities", "exp"].includes(key),
    ) ||
    !Array.isArray(editable.capabilities)
  ) {
    return null;
  }
  const capabilities = editable.capabilities.flatMap((value) => {
    if (typeof value !== "object" || value === null || Array.isArray(value)) {
      return [value];
    }
    const capability = value as Record<string, unknown>;
    if (
      capability.type === "filesystem:read" ||
      capability.type === "filesystem:write"
    ) {
      const customExcludes = capability.exclude;
      return [
        {
          ...capability,
          exclude:
            customExcludes === undefined || Array.isArray(customExcludes)
              ? [
                  ...new Set([
                    ...(Array.isArray(customExcludes) ? customExcludes : []),
                    ...REQUIRED_NXTLINQ_SENSITIVE_EXCLUDES,
                  ]),
                ]
              : customExcludes,
        },
      ];
    }
    if (capability.type === "mcp:connect") {
      if (!Array.isArray(capability.servers)) return [capability];
      const servers = capability.servers.filter(
        (server) => server !== "buzz-dev-mcp",
      );
      return servers.length > 0 ? [{ ...capability, servers }] : [];
    }
    return [capability];
  });
  return parseNxtlinqPolicyDraft({
    name: editable.name,
    version: editable.version,
    scope: ["demo:structured-capabilities"],
    aud: ["nxtlinq-authorization-gateway"],
    capabilities: [
      ...capabilities,
      { type: "mcp:connect", servers: ["buzz-dev-mcp"] },
    ],
    ...(editable.exp === undefined ? {} : { exp: editable.exp }),
  });
}

export function policyFromNxtlinqManifestJson(
  value: string,
): NxtlinqPolicyDraft | null {
  try {
    const manifest = JSON.parse(value) as Record<string, unknown>;
    return parseNxtlinqPolicyDraft({
      name: manifest.name,
      version: manifest.version,
      scope: manifest.scope,
      aud: manifest.aud,
      capabilities: manifest.capabilities,
      ...(manifest.exp === undefined ? {} : { exp: manifest.exp }),
    });
  } catch {
    return null;
  }
}
