// Mounted once in AppShell: keeps the remote-access status fresh and shows a local notification when a remote terminal opens.
import { invoke, isTauri } from "@tauri-apps/api/core";
import { readStoredNotificationSettings } from "@/features/notifications/hooks";
import { listen } from "@tauri-apps/api/event";
import { useQueryClient } from "@tanstack/react-query";
import * as React from "react";

import { discoverAcpRuntimes } from "@/shared/api/tauriAcpDiscovery";
import { createManagedAgent, listManagedAgents, updateManagedAgent } from "@/shared/api/tauri";
import { sendChannelMessage } from "@/shared/api/tauriMessages";
import { startManagedAgent, stopManagedAgent } from "@/shared/api/tauriManagedAgents";
import { managedAgentsQueryKey } from "@/features/agents/hooks";
import { listPersonas } from "@/shared/api/tauriPersonas";
import { CMD_WRITES } from "./directCmd";
import { handleMgmt, mgmtErrorString, type MgmtDeps } from "./directMgmt";
import { isAgentManagementAllowed, refreshRemoteAccess, subscribeRemoteAccess } from "./remoteAccessStore";

/** The real agent APIs behind direct-connection management requests. `pubkey` is the signed-in member (scope guard for start). */
export function realMgmtDeps(pubkey: string | undefined): MgmtDeps {
  return {
    allowed: isAgentManagementAllowed,
    listAgents: listManagedAgents,
    startAgent: (a) => startManagedAgent(a, { expectedSignerPubkey: pubkey }),
    stopAgent: stopManagedAgent,
    sendMessage: (a, channelId, text) =>
      sendChannelMessage(channelId, text, null, undefined, [a], undefined, undefined, undefined, undefined, undefined, undefined, pubkey),
    createAgent: createManagedAgent,
    updateAgent: updateManagedAgent,
    runtimes: () => discoverAcpRuntimes(),
    invokeRaw: (cmd, args) => invoke(cmd, args),
    ownerPubkey: pubkey,
    personas: async () => (await listPersonas()).map((p) => ({ id: p.id, displayName: p.displayName })),
  };
}

/** Ops that change agent state: the desktop's own agent lists (and the bridge snapshot) must refresh after them. */
const MUTATING_OPS = new Set(["agent.start", "agent.stop", "agent.create", "agent.rename", "agent.update"]);
const MUTATING_CMDS = new Set<string>(CMD_WRITES);
const isMutating = (op: string, params: unknown) =>
  MUTATING_OPS.has(op) || (op === "cmd" && MUTATING_CMDS.has(String((params as { name?: unknown } | null)?.name)));

export function useRemoteAccessEvents(pubkey?: string): void {
  const queryClient = useQueryClient();
  const queryClientRef = React.useRef(queryClient);
  queryClientRef.current = queryClient;
  const pubkeyRef = React.useRef(pubkey);
  pubkeyRef.current = pubkey;
  React.useEffect(() => {
    if (!isTauri()) return;
    const unsubStore = subscribeRemoteAccess(() => {});
    const unlistens: Array<() => void> = [];
    let cancelled = false;
    // Agent management over the direct connection: the Rust listener forwards each authenticated, switch-checked request here.
    void listen<{ reqId: string; op: string; params: unknown }>("remote-mgmt-request", async (e) => {
      const { reqId, op, params } = e.payload;
      try {
        const result = await handleMgmt(op, params, realMgmtDeps(pubkeyRef.current));
        await invoke("remote_mgmt_reply", { reqId, ok: true, result });
      } catch (err) {
        await invoke("remote_mgmt_reply", { reqId, ok: false, error: mgmtErrorString(err) }).catch(() => {});
      } finally {
        if (isMutating(op, params)) void queryClientRef.current.invalidateQueries({ queryKey: managedAgentsQueryKey });
      }
    }).then((u) => (cancelled ? u() : unlistens.push(u)));
    void listen("remote-access-changed", () => void refreshRemoteAccess()).then((u) => (cancelled ? u() : unlistens.push(u)));
    void listen<{ session?: string | null }>("remote-terminal-opened", async (e) => {
      void refreshRemoteAccess();
      const prefs = readStoredNotificationSettings(pubkeyRef.current?.trim().toLowerCase() ?? "");
      if (!prefs.desktopEnabled || !prefs.eventAlertsEnabled.remote_terminal) return;
      try {
        const { isPermissionGranted, sendNotification } = await import("@tauri-apps/plugin-notification");
        if (await isPermissionGranted())
          sendNotification({
            title: "Remote terminal opened",
            body: e.payload?.session ? `Your web app opened herdr session “${e.payload.session}” on this Mac.` : "Your web app opened a shell on this Mac.",
          });
      } catch {
        /* notifications unavailable */
      }
    }).then((u) => (cancelled ? u() : unlistens.push(u)));
    return () => {
      cancelled = true;
      unsubStore();
      for (const u of unlistens) u();
    };
  }, []);
}
