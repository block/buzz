# Buzz NixOS relay deployment

This guide covers deploying the Buzz relay with the NixOS module exported by
the flake. The module runs the `buzz-relay` systemd service; it does not
provision the full backing stack for you.

For a complete relay deployment, plan for:

- PostgreSQL: canonical event store, full-text search, channels, tokens,
  workflows, and audit log.
- Redis: pub/sub fan-out, presence, and typing indicators.
- S3-compatible object storage: media blobs, usually AWS S3, MinIO, or similar.
- Persistent local storage: git repository state under `services.buzz-relay.gitRepoPath`.
- Public DNS and TLS: clients should connect to a stable `wss://` relay URL.
- Stable secrets: relay identity, database credentials, S3 keys, and any git
  hook secrets.

## Import the module

Add the Buzz flake input and import the NixOS module:

```nix
{
  inputs.buzz.url = "github:block/buzz";

  outputs = { self, nixpkgs, buzz, ... }: {
    nixosConfigurations.relay = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        buzz.nixosModules.buzz-relay
        ./configuration.nix
      ];
    };
  };
}
```

## Relay service

Keep secrets out of normal Nix options when possible. Values in
`services.buzz-relay.environment` are written into the Nix store; use
`environmentFile` for passwords and private keys.

```nix
{ ... }:

{
  services.buzz-relay = {
    enable = true;

    relayUrl = "wss://buzz.example.com";
    host = "127.0.0.1";
    port = 3000;

    redisUrl = "redis://localhost:6379";
    s3Endpoint = "https://s3.example.com";
    s3Bucket = "buzz-media";

    environmentFile = "/run/secrets/buzz-relay.env";

    # Leave this true unless migrations are handled by a separate job.
    autoMigrate = true;
  };
}
```

When a value contains a secret, put the runtime environment variable in
`environmentFile` instead of setting the corresponding Nix option.

Example secret file:

```env
DATABASE_URL=postgres://buzz:CHANGE_ME@localhost:5432/buzz
BUZZ_RELAY_PRIVATE_KEY=CHANGE_ME_32_BYTE_HEX_PRIVATE_KEY
BUZZ_S3_ACCESS_KEY=CHANGE_ME
BUZZ_S3_SECRET_KEY=CHANGE_ME
BUZZ_GIT_HOOK_HMAC_SECRET=CHANGE_ME
```

`BUZZ_RELAY_PRIVATE_KEY` must stay stable across rebuilds and restores. Rotating
it gives the relay a new identity.

## Backing services

The NixOS module expects the backing services to exist. They can run on the same
host, in containers, or as managed external services.

Minimum required environment:

| Service | Required relay setting | Notes |
| --- | --- | --- |
| PostgreSQL | `DATABASE_URL` | Required. Run migrations before serving, or keep `autoMigrate = true`. |
| Redis | `REDIS_URL` | Required for cross-connection fan-out, presence, and typing. |
| S3-compatible storage | `BUZZ_S3_ENDPOINT`, `BUZZ_S3_BUCKET`, S3 keys | Required for media uploads. |
| Local filesystem | `gitRepoPath` | Persistent git repository state. Defaults under `/var/lib/buzz/git`. |

For a single-node NixOS host, it is reasonable to run PostgreSQL and Redis with
native NixOS services, and run MinIO as a container or external service. For
production, managed PostgreSQL, Redis, and S3 are easier to back up and upgrade.

## Reverse proxy and TLS

The relay serves HTTP and WebSocket traffic on one port. Put it behind a TLS
reverse proxy and advertise the public WebSocket URL with `relayUrl`.

Example with nginx:

```nix
{
  services.nginx = {
    enable = true;
    recommendedProxySettings = true;
    recommendedTlsSettings = true;

    virtualHosts."buzz.example.com" = {
      forceSSL = true;
      enableACME = true;

      locations."/" = {
        proxyPass = "http://127.0.0.1:3000";
        proxyWebsockets = true;
      };
    };
  };

  security.acme.acceptTerms = true;
  security.acme.defaults.email = "ops@example.com";
}
```

If the relay binds directly to a public interface instead, set
`services.buzz-relay.openFirewall = true;`. For a reverse proxy on the same host,
keep the relay bound to `127.0.0.1` and open only ports 80/443.

## Closed relay settings

For a closed relay, configure an owner and require membership:

```nix
{
  services.buzz-relay = {
    requireRelayMembership = true;
    ownerPubkey = "64_character_lowercase_hex_nostr_pubkey";
  };
}
```

`RELAY_OWNER_PUBKEY` is intentionally not named with a `BUZZ_` prefix in the
runtime environment. The module exposes it as `ownerPubkey`.

## Operations

Useful checks:

```bash
systemctl status buzz-relay
journalctl -u buzz-relay -f
curl -fsS http://127.0.0.1:8080/_liveness
curl -fsS http://127.0.0.1:8080/_readiness
```

The relay exposes:

- Main HTTP/WebSocket traffic on `services.buzz-relay.port` (`3000` by default).
- Health probes on `services.buzz-relay.healthPort` (`8080` by default).
- Prometheus metrics on `services.buzz-relay.metricsPort` (`9102` by default).

Back up these items before upgrades and before moving hosts:

- PostgreSQL database.
- S3 bucket contents.
- `services.buzz-relay.dataDir`, especially the git repository path.
- `BUZZ_RELAY_PRIVATE_KEY`.
- Owner private key, which is held by the operator, not by the relay.
- Secret files used by `environmentFile`.

If `autoMigrate = false`, run `buzz-admin migrate` from the configured package
against the database before starting a new relay version. For example, with
`DATABASE_URL` set:

```bash
nix shell github:block/buzz#buzz-runtime --command buzz-admin migrate
```

Pin that flake reference to the same revision as the deployed module. The NixOS
module does not create a separate migration job.
