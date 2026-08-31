import type { HostReport } from "./registration";

const availability = new Set([
  "available",
  "adapter_missing",
  "adapter_outdated",
  "cli_missing",
  "not_installed",
]);
const authStatus = new Set([
  "logged_in",
  "logged_out",
  "config_invalid",
  "not_applicable",
  "unknown",
]);
const encoder = new TextEncoder();
function text(value: unknown): value is string {
  return (
    typeof value === "string" &&
    value.length > 0 &&
    encoder.encode(value).length <= 256 &&
    !Array.from(value).some((c) => {
      const code = c.charCodeAt(0);
      return code < 32 || (code >= 127 && code <= 159);
    })
  );
}
function object(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
function fields(value: Record<string, unknown>, allowed: string[]) {
  return Object.keys(value).every((key) => allowed.includes(key));
}

/** Validate the decoded IPC payload before comparison, display or publication. */
export function validateHostReport(
  value: unknown,
): asserts value is HostReport {
  if (
    !object(value) ||
    !fields(value, [
      "v",
      "name",
      "os",
      "arch",
      "launcher_version",
      "runtimes",
      "accepts_start",
      "provisioned",
    ]) ||
    (value.v !== 1 && value.v !== 2 && value.v !== 3) ||
    typeof value.accepts_start !== "boolean" ||
    (value.v !== 3 &&
      (value.accepts_start ||
        (Array.isArray(value.provisioned) && value.provisioned.length > 0))) ||
    ![value.name, value.os, value.arch, value.launcher_version].every(text) ||
    !Array.isArray(value.runtimes) ||
    value.runtimes.length > 128
  )
    throw new Error("Invalid host report payload");
  const ids = new Set<string>();
  for (const runtime of value.runtimes) {
    if (
      !object(runtime) ||
      !fields(runtime, ["id", "label", "availability", "auth_status"]) ||
      !text(runtime.id) ||
      !text(runtime.label) ||
      !text(runtime.availability) ||
      !availability.has(runtime.availability) ||
      !text(runtime.auth_status) ||
      !authStatus.has(runtime.auth_status) ||
      ids.has(runtime.id)
    )
      throw new Error("Invalid host runtime payload");
    ids.add(runtime.id);
  }
  if (value.provisioned !== undefined) {
    if (!Array.isArray(value.provisioned) || value.provisioned.length > 256)
      throw new Error("Invalid host provisioning payload");
    const agents = new Set<string>();
    for (const config of value.provisioned) {
      if (
        !object(config) ||
        !fields(config, ["agent", "runtime", "revision"]) ||
        typeof config.agent !== "string" ||
        !/^[a-f0-9]{64}$/.test(config.agent) ||
        typeof config.revision !== "string" ||
        !/^[a-f0-9]{64}$/.test(config.revision) ||
        agents.has(config.agent) ||
        !value.runtimes.some(
          (r) =>
            r.id === config.runtime &&
            r.availability === "available" &&
            ["logged_in", "not_applicable"].includes(r.auth_status),
        )
      )
        throw new Error("Invalid host provisioning payload");
      agents.add(config.agent);
    }
  }
}
