# buzz-backend-hermes

Buzz Desktop provider for an existing, remotely supervised native Hermes
gateway.

The provider is discovered as `buzz-backend-hermes` and implements the Buzz
provider `info`/`deploy` protocol plus native-Hermes `stop` and authenticated
`cleanup` extensions. Cleanup bootouts the gateway and removes only the
provider-owned Buzz environment block before Desktop deletes the identity.
`deploy` sends the agent payload over SSH to the configured host, writes the
protected Hermes Buzz environment, applies `model.default` and
`model.provider`, and restarts the existing launchd/systemd unit. It never
starts a local ACP process and refuses non-Hermes agent commands.

Provider configuration is non-secret:

- `host` and explicit SSH `user`
- `profile`, `supervisor`, and `unit` (plus an optional launchd `plist` path)
- optional Hermes home/profile paths and executable paths
- Buzz channel UUIDs and home channel
- explicit `allowed_users` and `allow_all_users` relay authorization policy

SSH authentication is ambient (`ssh-agent`/user SSH configuration); no SSH
private key or Nostr secret belongs in `provider_config`. The Nostr private key
and NIP-OA auth tag arrive only in the Desktop deploy payload and are written
remotely with mode `0600`.

Relay authorization is explicit: `allow_all_users` defaults to false and
must be enabled in the provider configuration when the deployment policy is to
allow relay users, while `require_mention` remains enforced by the generated
configuration. The provider snapshots and restores the profile `.env` and
`config.yaml` if model configuration or supervisor restart fails.

This provider assumes the remote Hermes gateway and its supervisor already
exist. Deploy and stop operations take an exclusive per-profile remote lock. Configured Hermes home/profile paths must be canonical
(no symlink or dot-segment aliases), and the profile must remain beneath the
Hermes home. On launchd, stop uses `bootout` and deploy bootstraps/enables the plist again, so
KeepAlive cannot silently restart a stopped gateway.
Desktop also enforces one managed identity per `(host, profile, unit)` because
one supervised Hermes gateway is one lifecycle scope. It is therefore a remote
reconfiguration/deployment provider, not an identity importer that pretends an
unowned process is local.
