// Web command registry. Only commands the S1 chat path needs are implemented (or given a safe default);
// everything else is classified by capabilities.ts and rejects with a typed NativeOnlyError.
import { makeLinkPreviewHandlers } from "./linkPreview";
import { capabilityGroupFor, groupPolicy } from "./capabilities";
import { chatHandlers } from "./chat";
import { parityHandlers } from "./parity";
import { parity2Handlers } from "./parity2";
import { ownersFromProfiles } from "./nipOa";
import { getFeed, type FeedArgs } from "./feed";
import { queryRelay } from "./relayHttp";
import {
  defaultRelayWsUrl,
  relayHttpBase,
  relayWsUrl,
  setAppliedRelay,
} from "./relayConfig";
import {
  getRelayMediaBlobUrl,
  isRelayMediaUrl,
  pickFilesFromBrowser,
  uploadRelayMedia,
} from "./relayMedia";
import { installProxyHandlers } from "@/features/bridge/remoteCmd";
import { getSigner, signOutSigner, signerPubkey } from "./signer";

type Args = Record<string, any> | undefined;
type Handler = (
  args: Args,
  options?: { headers?: Record<string, string> },
) => unknown | Promise<unknown>;

function identityInfo() {
  const pubkey = signerPubkey();
  return {
    pubkey: pubkey ?? "",
    display_name: pubkey ? `web-${pubkey.slice(0, 8)}` : "",
    storage: "web-signer",
    ephemeral: false,
    locked: false,
    needs_import: pubkey === null,
  };
}

// Safe empty/default answers so the shell boots without desktop-only subsystems.
const EMPTY_LIST = [
  "list_managed_agents",
  "list_personas",
  "list_teams",
  "list_managed_agent_runtimes",
  "list_agent_cards",
  "list_save_subscriptions",
  "list_project_local_repositories",
  "list_channel_templates",
  "list_voice_registry",
  "list_audio_output_devices",
  "take_tray_actions",
  "browser_agent_grants_list",
  "list_relay_agents",
  "discover_acp_providers",
  "fetch_persona_catalog",
  "discover_backend_providers",
  "reconcile_managed_agent_runtimes",
];
const FALSE_CMDS = [
  "is_auto_update_supported",
  "relay_reconnect_hook_configured",
  "is_shared_identity",
  "agent_access_owner_only",
  "observer_archive_default_enabled",
  "agent_metric_archive_default_enabled",
  "huddle_companion_window_exists",
];
const NOOP = [
  "acknowledge_pending_community_deep_link",
  "acknowledge_pending_entity_deep_link",
  "acknowledge_pending_navigation_deep_link",
  "clear_pending_navigation_deep_links",
  "relay_reconnect_hook",
  "clear_tray_agent_activity",
  "update_tray_agent_activity",
  "persist_current_identity",
  "set_agent_avatar_communities",
  "remove_community_relay",
  "readd_community_relay",
  "set_prevent_sleep_active",
  "merge_save_subscription_kinds",
];
const NULL_CMDS = [
  "get_os_idle_seconds",
  "get_huddle_state",
  "get_relay_self",
  "take_pending_community_deep_link",
  "take_pending_entity_deep_link",
  "take_pending_navigation_deep_link",
];

export const handlers: Record<string, Handler> = {
  // relay + identity
  get_default_relay_url: defaultRelayWsUrl,
  get_relay_ws_url: relayWsUrl,
  get_relay_http_url: relayHttpBase,
  auto_connect_default_relay_enabled: () => true, // web build always auto-connects to the compiled default relay
  apply_workspace: (a) => {
    setAppliedRelay(a?.relayUrl ?? null);
  },
  get_identity: identityInfo,
  sign_out: async () => {
    await signOutSigner();
    location.reload();
  },
  // signing — always delegated to the Signer (NIP-07 extension)
  sign_event: async (a) =>
    JSON.stringify(
      await getSigner().signEvent({
        kind: a?.kind,
        content: a?.content ?? "",
        tags: a?.tags ?? [],
        created_at: a?.createdAt ?? undefined,
      }),
    ),
  create_auth_event: async (a) =>
    JSON.stringify(
      await getSigner().signEvent({
        kind: 22242,
        content: "",
        tags: [
          ["relay", a?.relayUrl],
          ["challenge", a?.challenge],
        ],
      }),
    ),
  nip44_encrypt_to_self: async (a) => {
    const n = getSigner().nip44;
    if (!n) throw new Error("Your signer does not support NIP-44 encryption.");
    return n.encrypt(signerPubkey() as string, a?.plaintext ?? "");
  },
  nip44_decrypt_from_self: async (a) => {
    const n = getSigner().nip44;
    if (!n) throw new Error("Your signer does not support NIP-44 decryption.");
    return n.decrypt(signerPubkey() as string, a?.ciphertext ?? "");
  },
  // build/env
  get_baked_build_env: () => [],
  get_baked_build_env_keys: () => [],
  get_media_proxy_port: () => 0,
  // media upload (PUT /upload with a Blossom t=upload token via the signer; no native dialogs/IPC)
  upload_media_bytes: (a) =>
    uploadRelayMedia(Uint8Array.from(a?.data ?? []), { filename: a?.filename }),
  upload_media_bytes_raw: (a, o) => {
    const h = o?.headers?.["x-buzz-filename"];
    let filename: string | undefined;
    try {
      if (h) filename = new TextDecoder().decode(Uint8Array.from(atob(h.replaceAll("-", "+").replaceAll("_", "/")), (c) => c.charCodeAt(0)));
    } catch {
      /* ignore bad header */
    }
    return uploadRelayMedia(a as unknown as Uint8Array, { filename });
  },
  pick_and_upload_media: async () => {
    const files = await pickFilesFromBrowser();
    const out = [];
    for (const f of files)
      out.push(
        await uploadRelayMedia(new Uint8Array(await f.arrayBuffer()), {
          filename: f.name,
          mime: f.type,
        }),
      );
    return out;
  },
  pick_and_upload_image: async () => {
    const [f] = await pickFilesFromBrowser("image/*");
    return f
      ? uploadRelayMedia(new Uint8Array(await f.arrayBuffer()), {
          filename: f.name,
          mime: f.type,
        })
      : null;
  },
  // composer image editor: source bytes for the canvas (signed relay fetch -> blob: -> bytes)
  fetch_media_bytes: async (a) => {
    const url = String(a?.url ?? "");
    const src = isRelayMediaUrl(url) ? await getRelayMediaBlobUrl(url) : url;
    const res = await fetch(src);
    if (!res.ok) throw new Error(`Could not load image (${res.status})`);
    return res.arrayBuffer();
  },
  cancel_media_fetch: () => undefined,
  cancel_media_upload: () => undefined,
  release_media_upload: () => undefined,
  get_legacy_workspace_storage: () => ({
    workspaces: null,
    activeWorkspaceId: null,
    onboardingCompletions: [],
  }),
  // browser equivalents
  copy_text_to_clipboard: (a) => navigator.clipboard.writeText(a?.text ?? ""),
  read_clipboard_text: () => navigator.clipboard.readText(),
  // quiet defaults for desktop-only subsystems the shell touches at boot
  // Home feed: mentions + approvals (+ edits for link-preview suppression), ported from desktop get_feed.
  get_feed: (a) => {
    const me = signerPubkey();
    if (!me)
      return {
        feed: { mentions: [], needs_action: [], activity: [], agent_activity: [] },
        meta: { since: 0, total: 0, generated_at: Math.floor(Date.now() / 1000) },
      };
    return getFeed(a as FeedArgs, me, (filters) => queryRelay(filters as never) as never, (p) => ownersFromProfiles(p as never));
  },
  get_voice_input_mode: () => "push_to_talk",
  get_audio_output_device: () => "",
  // chat path (S1)
  ...chatHandlers,
  // web parity round 1: search, edit, channel management, downloads
  ...parityHandlers,
};
for (const c of EMPTY_LIST) handlers[c] = () => [];
for (const c of FALSE_CMDS) handlers[c] = () => false;
for (const c of NOOP) handlers[c] = () => undefined;
for (const c of NULL_CMDS) handlers[c] = () => null;
// Desktop agent data/commands: answered by the desktop over the direct connection (see features/bridge/remoteCmd.ts).
installProxyHandlers(handlers);
// Object-shaped placeholder (desktop returns { archived: string[] }); an empty array crashed the profile panel.
handlers.list_archived_identities = () => ({ archived: [] });
// web parity round 2 (pure relay ports) take precedence over the placeholder lists above
Object.assign(handlers, parity2Handlers);
// Link preview cards: only with a configured helper (server/link-preview); otherwise previews stay desktop-only.
if (import.meta.env.VITE_BUZZ_LINK_PREVIEW_URL) Object.assign(handlers, makeLinkPreviewHandlers(String(import.meta.env.VITE_BUZZ_LINK_PREVIEW_URL)));

export class NativeOnlyError extends Error {
  constructor(
    public command: string,
    public group: string,
    public policy: string,
  ) {
    super(
      `"${command}" is not available in the web app (${group}); ${policy === "proxy" ? "needs a paired desktop/server" : "available in the desktop app"}`,
    );
    this.name = "NativeOnlyError";
  }
}

export function nativeOnly(command: string): NativeOnlyError {
  const g = capabilityGroupFor(command);
  return new NativeOnlyError(command, g.id, groupPolicy(g.id));
}
