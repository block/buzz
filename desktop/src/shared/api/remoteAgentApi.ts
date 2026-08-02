import { invokeTauri } from "@/shared/api/tauri";
import type { HostProbeResult, SshHost } from "@/shared/api/remoteAgentTypes";

/**
 * Enumerate the user's `~/.ssh/config` host aliases. No connection is attempted;
 * an absent config yields an empty list.
 */
export async function listSshHosts(): Promise<SshHost[]> {
  return await invokeTauri<SshHost[]>("list_ssh_hosts");
}

/**
 * Probe one configured host for agent harnesses and the `buzz` CLI.
 *
 * `host` must be an alias present in `~/.ssh/config`. A host-side failure
 * (unreachable, password-only, unknown key) resolves with `ok: false` and a
 * classified `errorKind`; only a failure to run `ssh` at all rejects.
 */
export async function probeAgentHost(host: string): Promise<HostProbeResult> {
  return await invokeTauri<HostProbeResult>("probe_agent_host", { host });
}

/**
 * Probe the machine Buzz is running on, using the identical probe script so the
 * result is shape-compatible with `probeAgentHost`.
 */
export async function probeLocalAgentHost(): Promise<HostProbeResult> {
  return await invokeTauri<HostProbeResult>("probe_local_agent_host");
}
