# Buzz on NixOS

The repository flake builds the Buzz desktop app, relay server, administration
CLI, and agent sidecars. It also exports a NixOS module for running the relay as
a hardened systemd service.

## Build and run

From a Buzz checkout:

```bash
nix build .#buzz-desktop
nix build .#buzz-relay
nix build .#buzz-sidecars

nix run .#buzz-desktop
nix run .#buzz-relay
nix run .#buzz-admin -- --help
```

The `buzz-relay` package contains both `buzz-relay` and `buzz-admin`. The named
apps select the appropriate executable.

## Import the NixOS module

Pin Buzz as a flake input and import its relay module:

```nix
{
  inputs.buzz.url = "github:block/buzz";

  outputs =
    {
      nixpkgs,
      buzz,
      ...
    }:
    {
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

The module supports x86-64 and ARM64 Linux.

## Configure the relay

The module runs the relay but does not provision PostgreSQL, Redis, object
storage, DNS, or TLS. Those services may run on the same host or be provided
externally.

```nix
{
  services.buzz-relay = {
    enable = true;

    relayUrl = "wss://buzz.example.com";
    host = "127.0.0.1";
    port = 3000;

    databaseUrl = "postgres://buzz@localhost:5432/buzz";
    redisUrl = "redis://localhost:6379";
    s3Endpoint = "https://s3.example.com";
    s3Bucket = "buzz-media";
    s3Region = "eu-west-2";
    s3AddressingStyle = "path";
    mediaBaseUrl = "https://buzz.example.com/media";

    environmentFile = "/run/secrets/buzz-relay.env";
    autoMigrate = true;
  };
}
```

URLs containing credentials can be supplied through `environmentFile` instead
of normal Nix options, which are copied into the Nix store.

Example runtime secret file:

```env
DATABASE_URL=postgres://buzz:CHANGE_ME@localhost:5432/buzz
BUZZ_RELAY_PRIVATE_KEY=CHANGE_ME_32_BYTE_HEX_PRIVATE_KEY
BUZZ_S3_ACCESS_KEY=CHANGE_ME
BUZZ_S3_SECRET_KEY=CHANGE_ME
BUZZ_GIT_HOOK_HMAC_SECRET=CHANGE_ME_AT_LEAST_32_CHARACTERS
```

Keep `BUZZ_RELAY_PRIVATE_KEY` stable across rebuilds and restores. A changed key
gives the relay a new identity.

Unmodeled relay settings can be passed through `environment` when they are not
secret:

```nix
{
  services.buzz-relay.environment = {
    BUZZ_RATE_LIMIT_HUMAN_MESSAGES_PER_MIN = 120;
    BUZZ_MEDIA_UPLOADS_PER_MINUTE = 20;
  };
}
```

Deployments with a Postgres read replica can size its pool independently and
opt into bounded-staleness routing:

```nix
{
  services.buzz-relay = {
    readDatabaseUrl = "postgres://buzz@reader.example.com:5432/buzz";
    readDatabasePoolSize = 25;
    replicaReadMaxAgeMs = 1000;
  };
}
```

Replica routing remains disabled when `replicaReadMaxAgeMs` is zero.

## Closed relays

Membership enforcement requires a stable relay key and an owner pubkey:

```nix
{
  services.buzz-relay = {
    requireRelayMembership = true;
    ownerPubkey = "64_character_lowercase_hex_nostr_pubkey";
    environmentFile = "/run/secrets/buzz-relay.env";
  };
}
```

The environment file must provide `BUZZ_RELAY_PRIVATE_KEY`.

## Reverse proxy

The public HTTP and WebSocket traffic shares one port. A typical deployment
binds Buzz to localhost and terminates TLS with nginx:

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

  security.acme = {
    acceptTerms = true;
    defaults.email = "ops@example.com";
  };
}
```

`openFirewall` opens only the public relay port. Health and metrics ports remain
private unless configured separately.

## Operations

The module creates a `buzz-relay` system user and persists git data beneath
`dataDir`, which defaults to `/var/lib/buzz`.

```bash
systemctl status buzz-relay
journalctl -u buzz-relay -f
curl --fail http://127.0.0.1:8080/_liveness
curl --fail http://127.0.0.1:8080/_readiness
```

Prometheus metrics are exposed on port `9102` by default.

For horizontally scaled relay deployments, disable in-process huddle audio:

```nix
services.buzz-relay.huddleAudioAvailable = false;
```

Back up PostgreSQL, the S3 bucket, `dataDir`, and all secret files before
upgrades. If `autoMigrate` is disabled, run `buzz-admin migrate` from the same
pinned Buzz revision before starting the new relay:

```bash
nix run github:block/buzz#buzz-admin -- migrate
```
