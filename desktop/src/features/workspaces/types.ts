/**
 * Workspace transport mode.
 *
 * - `sprout` — the relay is a Sprout server (Postgres, auth, search, HTTP
 *   bridge, NIP-42 AUTH required). The default for all existing workspaces.
 * - `serverless` — the relay is a generic public Nostr relay. No Sprout server
 *   infrastructure: reads/writes go over plain WebSocket REQ/EVENT, AUTH is
 *   optional, channel membership is advisory (not enforced), and server-only
 *   features (search, presence fan-out, huddle, git hosting) are hidden.
 *   See docs/SPROUT_LITE_MODE.md.
 */
export type WorkspaceMode = "sprout" | "serverless";

export type Workspace = {
  id: string;
  name: string;
  relayUrl: string;
  token?: string;
  /**
   * Transport mode. Absent on legacy entries — treat `undefined` as
   * `"sprout"` everywhere (see `workspaceMode()` in workspaceStorage.ts).
   */
  mode?: WorkspaceMode;
  /**
   * The pubkey associated with the active identity at the time the workspace
   * was created. Display-only — auth always uses the persisted `identity.key`
   * file resolved at startup, never this field.
   */
  pubkey?: string;
  addedAt: string;
  /**
   * @deprecated Never read. Kept on the type so old localStorage entries
   * deserialise without errors. New entries never set this field, and
   * `loadWorkspaces()` strips it on read so it cannot leak forward. The
   * authoritative private key is the on-disk `identity.key` file.
   */
  nsec?: never;
};
