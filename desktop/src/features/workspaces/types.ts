export type Workspace = {
  id: string;
  name: string;
  relayUrl: string;
  token?: string;
  /**
   * The pubkey associated with the active identity at the time the workspace
   * was created. Display-only — auth always uses the persisted `identity.key`
   * file resolved at startup, never this field.
   */
  pubkey?: string;
  addedAt: string;
  /**
   * Absolute directory the agent's `~/.buzz/REPOS` symlinks to, so agents
   * work in the user's existing checkouts instead of re-cloning. `~` is
   * expanded to an absolute path before save. Unset = the default real
   * `REPOS` directory inside the nest.
   */
  reposDir?: string;
  /**
   * @deprecated Never read. Kept on the type so old localStorage entries
   * deserialise without errors. New entries never set this field, and
   * `loadWorkspaces()` strips it on read so it cannot leak forward. The
   * authoritative private key is the on-disk `identity.key` file.
   */
  nsec?: never;
};
