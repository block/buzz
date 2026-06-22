{ self }:
{ config, lib, pkgs, ... }:

let
  cfg = config.services.buzz-relay;

  inherit (lib)
    boolToString
    concatStringsSep
    mkEnableOption
    mkIf
    mkOption
    optionalAttrs
    types;

  envValue = value:
    if builtins.isBool value then boolToString value
    else toString value;

  nullableEnv = name: value:
    optionalAttrs (value != null) { ${name} = value; };

  bindHost =
    if lib.hasPrefix "[" cfg.host && lib.hasSuffix "]" cfg.host then cfg.host
    else if lib.hasInfix ":" cfg.host then "[${cfg.host}]"
    else cfg.host;

  relayEnv =
    {
      BUZZ_BIND_ADDR = "${bindHost}:${toString cfg.port}";
      BUZZ_HEALTH_PORT = cfg.healthPort;
      BUZZ_METRICS_PORT = cfg.metricsPort;
      BUZZ_MAX_CONNECTIONS = cfg.maxConnections;
      BUZZ_MAX_CONCURRENT_HANDLERS = cfg.maxConcurrentHandlers;
      BUZZ_SEND_BUFFER = cfg.sendBuffer;
      BUZZ_REQUIRE_AUTH_TOKEN = cfg.requireAuthToken;
      BUZZ_REQUIRE_RELAY_MEMBERSHIP = cfg.requireRelayMembership;
      BUZZ_ALLOW_NIP_OA_AUTH = cfg.allowNipOaAuth;
      BUZZ_PUBKEY_ALLOWLIST = cfg.pubkeyAllowlist;
      BUZZ_AUTO_MIGRATE = cfg.autoMigrate;
      BUZZ_GIT_REPO_PATH = cfg.gitRepoPath;
      BUZZ_GIT_MAX_PACK_BYTES = cfg.git.maxPackBytes;
      BUZZ_GIT_MAX_REPOS_PER_PUBKEY = cfg.git.maxReposPerPubkey;
      BUZZ_GIT_MAX_CONCURRENT_OPS = cfg.git.maxConcurrentOps;
      RUST_LOG = cfg.logFilter;
    }
    // nullableEnv "RELAY_URL" cfg.relayUrl
    // nullableEnv "RELAY_OWNER_PUBKEY" cfg.ownerPubkey
    // nullableEnv "DATABASE_URL" cfg.databaseUrl
    // nullableEnv "REDIS_URL" cfg.redisUrl
    // nullableEnv "BUZZ_S3_ENDPOINT" cfg.s3Endpoint
    // nullableEnv "BUZZ_S3_BUCKET" cfg.s3Bucket
    // nullableEnv "BUZZ_MEDIA_BASE_URL" cfg.mediaBaseUrl
    // nullableEnv "BUZZ_MEDIA_SERVER_DOMAIN" cfg.mediaServerDomain
    // nullableEnv "BUZZ_WEB_DIR" cfg.webDir
    // optionalAttrs (cfg.corsOrigins != [ ]) {
      BUZZ_CORS_ORIGINS = concatStringsSep "," cfg.corsOrigins;
    }
    // optionalAttrs (cfg.ephemeralTtlOverride != null) {
      BUZZ_EPHEMERAL_TTL_OVERRIDE = cfg.ephemeralTtlOverride;
    };
in
{
  options.services.buzz-relay = {
    enable = mkEnableOption "Buzz relay";

    package = mkOption {
      type = types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.buzz-runtime;
      defaultText = lib.literalExpression "inputs.buzz.packages.\${pkgs.stdenv.hostPlatform.system}.buzz-runtime";
      description = "Package providing the buzz-relay binary.";
    };

    user = mkOption {
      type = types.str;
      default = "buzz-relay";
      description = "User account that runs the relay.";
    };

    group = mkOption {
      type = types.str;
      default = "buzz-relay";
      description = "Group account that runs the relay.";
    };

    dataDir = mkOption {
      type = types.path;
      default = "/var/lib/buzz";
      description = "Persistent state directory for relay-managed local data.";
    };

    gitRepoPath = mkOption {
      type = types.path;
      default = "${cfg.dataDir}/git";
      defaultText = lib.literalExpression ''"${config.services.buzz-relay.dataDir}/git"'';
      description = "Directory used by the relay for git repository state.";
    };

    host = mkOption {
      type = types.str;
      default = "0.0.0.0";
      description = "Address the relay HTTP/WebSocket server binds to.";
    };

    port = mkOption {
      type = types.port;
      default = 3000;
      description = "TCP port for the relay HTTP/WebSocket server.";
    };

    healthPort = mkOption {
      type = types.port;
      default = 8080;
      description = "TCP port for relay liveness, readiness, and status probes.";
    };

    metricsPort = mkOption {
      type = types.port;
      default = 9102;
      description = "TCP port for Prometheus metrics.";
    };

    relayUrl = mkOption {
      type = types.nullOr types.str;
      default = null;
      example = "wss://buzz.example.com";
      description = "Public WebSocket URL advertised by the relay.";
    };

    mediaBaseUrl = mkOption {
      type = types.nullOr types.str;
      default = null;
      example = "https://buzz.example.com/media";
      description = "Public base URL for media served by the relay.";
    };

    mediaServerDomain = mkOption {
      type = types.nullOr types.str;
      default = null;
      example = "buzz.example.com";
      description = "Domain clients should associate with relay-hosted media.";
    };

    ownerPubkey = mkOption {
      type = types.nullOr types.str;
      default = null;
      example = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
      description = "Optional 64-character hex Nostr pubkey to bootstrap as relay owner.";
    };

    databaseUrl = mkOption {
      type = types.nullOr types.str;
      default = null;
      example = "postgres://buzz:buzz@localhost:5432/buzz";
      description = "Postgres connection URL. Prefer environmentFile when it contains a password.";
    };

    redisUrl = mkOption {
      type = types.nullOr types.str;
      default = null;
      example = "redis://localhost:6379";
      description = "Redis URL used for pub/sub fan-out.";
    };

    s3Endpoint = mkOption {
      type = types.nullOr types.str;
      default = null;
      example = "http://localhost:9000";
      description = "S3-compatible object storage endpoint for media.";
    };

    s3Bucket = mkOption {
      type = types.nullOr types.str;
      default = null;
      example = "buzz-media";
      description = "S3 bucket name for media.";
    };

    webDir = mkOption {
      type = types.nullOr types.path;
      default = null;
      description = "Optional built web UI directory containing index.html.";
    };

    requireAuthToken = mkOption {
      type = types.bool;
      default = true;
      description = "Whether REST API requests must present a valid token.";
    };

    requireRelayMembership = mkOption {
      type = types.bool;
      default = false;
      description = "Whether authenticated requests must pass relay membership checks.";
    };

    allowNipOaAuth = mkOption {
      type = types.bool;
      default = false;
      description = "Whether NIP-OA owner attestation can grant membership access on closed relays.";
    };

    pubkeyAllowlist = mkOption {
      type = types.bool;
      default = false;
      description = "Whether NIP-42 pubkey-only auth is restricted to the pubkey allowlist.";
    };

    autoMigrate = mkOption {
      type = types.bool;
      default = true;
      description = "Whether buzz-relay runs embedded SQL migrations at startup.";
    };

    maxConnections = mkOption {
      type = types.ints.positive;
      default = 10000;
      description = "Maximum number of concurrent WebSocket connections.";
    };

    maxConcurrentHandlers = mkOption {
      type = types.ints.positive;
      default = 1024;
      description = "Maximum number of concurrently executing message handlers.";
    };

    sendBuffer = mkOption {
      type = types.ints.positive;
      default = 1000;
      description = "Per-connection outbound message buffer size.";
    };

    corsOrigins = mkOption {
      type = types.listOf types.str;
      default = [ ];
      example = [ "https://buzz.example.com" "tauri://localhost" ];
      description = "Allowed CORS origins. Empty keeps the relay's permissive development default.";
    };

    ephemeralTtlOverride = mkOption {
      type = types.nullOr types.ints.positive;
      default = null;
      description = "Optional TTL override, in seconds, for ephemeral channels.";
    };

    git = {
      maxPackBytes = mkOption {
        type = types.ints.positive;
        default = 500 * 1024 * 1024;
        description = "Maximum accepted git pack size in bytes.";
      };

      maxReposPerPubkey = mkOption {
        type = types.ints.positive;
        default = 100;
        description = "Maximum number of git repositories per pubkey.";
      };

      maxConcurrentOps = mkOption {
        type = types.ints.positive;
        default = 20;
        description = "Maximum number of concurrent git subprocess operations.";
      };
    };

    logFilter = mkOption {
      type = types.str;
      default = "info,buzz_relay=info";
      description = "RUST_LOG filter for the relay service.";
    };

    environment = mkOption {
      type = types.attrsOf (types.oneOf [ types.str types.int types.bool types.path ]);
      default = { };
      example = {
        BUZZ_RELAY_PRIVATE_KEY = "32-byte-hex-private-key";
      };
      description = ''
        Extra environment variables for buzz-relay.

        Values here are written into the Nix store. Prefer environmentFile for
        secrets such as DATABASE_URL, BUZZ_RELAY_PRIVATE_KEY,
        BUZZ_GIT_HOOK_HMAC_SECRET, BUZZ_S3_ACCESS_KEY, and BUZZ_S3_SECRET_KEY.
      '';
    };

    environmentFile = mkOption {
      type = types.nullOr (types.either types.path (types.listOf types.path));
      default = null;
      example = "/run/secrets/buzz-relay.env";
      description = "Environment file or files read by systemd for secrets and deployment-specific overrides.";
    };

    path = mkOption {
      type = types.listOf types.package;
      default = with pkgs; [ git openssl curl ];
      defaultText = lib.literalExpression "with pkgs; [ git openssl curl ]";
      description = "Packages added to PATH for relay-managed git subprocesses and hooks.";
    };

    extraReadWritePaths = mkOption {
      type = types.listOf types.path;
      default = [ ];
      description = "Additional paths the hardened systemd unit may write to.";
    };

    openFirewall = mkOption {
      type = types.bool;
      default = false;
      description = "Whether to open the relay port in the NixOS firewall.";
    };
  };

  config = mkIf cfg.enable {
    users.groups.${cfg.group} = { };
    users.users.${cfg.user} = {
      isSystemUser = true;
      group = cfg.group;
      home = cfg.dataDir;
    };

    systemd.tmpfiles.rules = [
      "d ${cfg.dataDir} 0750 ${cfg.user} ${cfg.group} - -"
      "d ${cfg.gitRepoPath} 0750 ${cfg.user} ${cfg.group} - -"
    ];

    systemd.services.buzz-relay = {
      description = "Buzz relay";
      wantedBy = [ "multi-user.target" ];
      wants = [ "network-online.target" ];
      after = [ "network-online.target" ];

      path = cfg.path;
      environment = builtins.mapAttrs (_: envValue) (relayEnv // cfg.environment);

      serviceConfig = {
        ExecStart = "${cfg.package}/bin/buzz-relay";
        User = cfg.user;
        Group = cfg.group;
        WorkingDirectory = cfg.dataDir;
        Restart = "on-failure";
        RestartSec = "5s";

        NoNewPrivileges = true;
        PrivateTmp = true;
        ProtectHome = true;
        ProtectSystem = "strict";
        ReadWritePaths = [ cfg.dataDir cfg.gitRepoPath ] ++ cfg.extraReadWritePaths;
      } // optionalAttrs (cfg.environmentFile != null) {
        EnvironmentFile = cfg.environmentFile;
      };
    };

    networking.firewall.allowedTCPPorts = lib.optional cfg.openFirewall cfg.port;
  };
}
