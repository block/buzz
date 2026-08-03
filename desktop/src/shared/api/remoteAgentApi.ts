import { invokeTauri } from "@/shared/api/tauri";
import type {
  ConnectedAgent,
  HostProbeResult,
  SshHost,
} from "@/shared/api/remoteAgentTypes";

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

/** The self-hosted agents this machine is connected to. */
export async function listConnectedAgents(): Promise<ConnectedAgent[]> {
  return await invokeTauri<ConnectedAgent[]>("list_connected_agents");
}

/**
 * Record a self-hosted agent that already runs on `host`.
 *
 * `pubkey` accepts an npub or 64 hex characters and is normalized to hex by the
 * backend. An nsec is refused with a specific message — a self-hosted agent's
 * secret must never leave its own machine, and this call never transports one.
 * `host` must be an alias present in `~/.ssh/config`, because it is also the
 * reachability probe target.
 */
export async function connectRemoteAgent(input: {
  host: string;
  pubkey: string;
  name: string;
  harness?: string | null;
}): Promise<ConnectedAgent> {
  return await invokeTauri<ConnectedAgent>("connect_remote_agent", {
    host: input.host,
    pubkey: input.pubkey,
    name: input.name,
    harness: input.harness ?? null,
  });
}

/**
 * Forget a connected agent.
 *
 * Local-only: this removes Buzz's pointer and nothing else. The remote process
 * keeps running, and no tombstone or archive event is published — Buzz never
 * claimed to own this agent, so it has nothing to revoke.
 */
export async function disconnectRemoteAgent(pubkey: string): Promise<void> {
  await invokeTauri<void>("disconnect_remote_agent", { pubkey });
}
