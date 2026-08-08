{ self }:
{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.services.buzz-relay;

  inherit (lib)
    boolToString
    concatStringsSep
    mkEnableOption
    mkIf
    mkOption
    optionalAttrs
    types
    ;

  envValue = value: if builtins.isBool value then boolToString value else toString value;

  nullableEnv = name: value: optionalAttrs (value != null) { ${name} = value; };

  bindHost =
    if lib.hasPrefix "[" cfg.host && lib.hasSuffix "]" cfg.host then
      cfg.host
    else if lib.hasInfix ":" cfg.host then
      "[${cfg.host}]"
    else
      cfg.host;

  relayEnvironment = {
    BUZZ_BIND_ADDR = "${bindHost}:${toString cfg.port}";
    BUZZ_HEALTH_PORT = cfg.healthPort;
    BUZZ_METRICS_PORT = cfg.metricsPort;
    BUZZ_REDIS_POOL_SIZE = cfg.redisPoolSize;
    BUZZ_DB_POOL_SIZE = cfg.databasePoolSize;
    BUZZ_REPLICA_READ_MAX_AGE_MS = cfg.replicaReadMaxAgeMs;
    BUZZ_MAX_CONNECTIONS = cfg.maxConnections;
    BUZZ_MAX_CONCURRENT_HANDLERS = cfg.maxConcurrentHandlers;
    BUZZ_SEND_BUFFER = cfg.sendBuffer;
    BUZZ_MAX_FRAME_BYTES = cfg.maxFrameBytes;
    BUZZ_SLOW_CLIENT_GRACE_LIMIT = cfg.slowClientGraceLimit;
    BUZZ_REQUIRE_AUTH_TOKEN = cfg.requireAuthToken;
    BUZZ_REQUIRE_RELAY_MEMBERSHIP = cfg.requireRelayMembership;
    BUZZ_ALLOW_NIP_OA_AUTH = cfg.allowNipOaAuth;
    BUZZ_PUBKEY_ALLOWLIST = cfg.pubkeyAllowlist;
    BUZZ_REQUIRE_MEDIA_GET_AUTH = cfg.requireMediaGetAuth;
    BUZZ_HUDDLE_AUDIO_AVAILABLE = cfg.huddleAudioAvailable;
    BUZZ_AUDIT_ENABLED = cfg.auditEnabled;
    BUZZ_AUTO_MIGRATE = cfg.autoMigrate;
    BUZZ_GIT_REPO_PATH = cfg.gitRepoPath;
    BUZZ_GIT_PACK_CACHE_PATH = cfg.git.packCachePath;
    BUZZ_GIT_MAX_PACK_BYTES = cfg.git.maxPackBytes;
    BUZZ_GIT_MAX_REPO_BYTES = cfg.git.maxRepoBytes;
    BUZZ_GIT_PACK_CACHE_MAX_BYTES = cfg.git.packCacheMaxBytes;
    BUZZ_GIT_PACK_CACHE_MAX_CONCURRENT_POPULATIONS = cfg.git.packCacheMaxConcurrentPopulations;
    BUZZ_GIT_MAX_REPOS_PER_PUBKEY = cfg.git.maxReposPerPubkey;
    BUZZ_GIT_MAX_CONCURRENT_OPS = cfg.git.maxConcurrentOps;
    RUST_LOG = cfg.logFilter;
  }
  // nullableEnv "RELAY_URL" cfg.relayUrl
  // nullableEnv "RELAY_OWNER_PUBKEY" cfg.ownerPubkey
  // nullableEnv "DATABASE_URL" cfg.databaseUrl
  // nullableEnv "READ_DATABASE_URL" cfg.readDatabaseUrl
  // nullableEnv "BUZZ_DB_READ_POOL_SIZE" cfg.readDatabasePoolSize
  // nullableEnv "REDIS_URL" cfg.redisUrl
  // nullableEnv "BUZZ_S3_ENDPOINT" cfg.s3Endpoint
  // nullableEnv "BUZZ_S3_BUCKET" cfg.s3Bucket
  // nullableEnv "BUZZ_S3_REGION" cfg.s3Region
  // nullableEnv "BUZZ_S3_ADDRESSING_STYLE" cfg.s3AddressingStyle
  // nullableEnv "BUZZ_MEDIA_BASE_URL" cfg.mediaBaseUrl
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
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.buzz-relay;
      defaultText = lib.literalExpression "inputs.buzz.packages.\${pkgs.system}.buzz-relay";
      description = "Package providing the buzz-relay and buzz-admin binaries.";
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
      defaultText = lib.literalExpression ''"''${config.services.buzz-relay.dataDir}/git"'';
      description = "Directory used for git repository state.";
    };

    host = mkOption {
      type = types.str;
      default = "0.0.0.0";
      description = "Address on which the relay HTTP and WebSocket server listens.";
    };

    port = mkOption {
      type = types.port;
      default = 3000;
      description = "Port for relay HTTP and WebSocket traffic.";
    };

    healthPort = mkOption {
      type = types.port;
      default = 8080;
      description = "Port for relay liveness and readiness probes.";
    };

    metricsPort = mkOption {
      type = types.port;
      default = 9102;
      description = "Port for Prometheus metrics.";
    };

    relayUrl = mkOption {
      type = types.nullOr types.str;
      default = null;
      example = "wss://buzz.example.com";
      description = "Public WebSocket URL advertised by the relay.";
    };

    ownerPubkey = mkOption {
      type = types.nullOr types.str;
      default = null;
      example = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
      description = "Optional Nostr pubkey to bootstrap as relay owner.";
    };

    databaseUrl = mkOption {
      type = types.nullOr types.str;
      default = null;
      example = "postgres://buzz@localhost:5432/buzz";
      description = "Postgres writer URL. Prefer environmentFile when it contains credentials.";
    };

    readDatabaseUrl = mkOption {
      type = types.nullOr types.str;
      default = null;
      example = "postgres://buzz@reader.example.com:5432/buzz";
      description = "Optional Postgres read-replica URL.";
    };

    redisUrl = mkOption {
      type = types.nullOr types.str;
      default = null;
      example = "redis://localhost:6379";
      description = "Redis URL used for pub/sub and ephemeral state.";
    };

    redisPoolSize = mkOption {
      type = types.ints.positive;
      default = 16;
      description = "Maximum number of connections in the Redis pool.";
    };

    databasePoolSize = mkOption {
      type = types.ints.positive;
      default = 50;
      description = "Maximum writer-pool size and default read-replica pool size.";
    };

    readDatabasePoolSize = mkOption {
      type = types.nullOr types.ints.positive;
      default = null;
      description = "Optional independent Postgres read-replica pool size. Null inherits databasePoolSize.";
    };

    replicaReadMaxAgeMs = mkOption {
      type = types.ints.unsigned;
      default = 0;
      description = "Maximum replica snapshot age in milliseconds. Zero disables replica routing.";
    };

    s3Endpoint = mkOption {
      type = types.nullOr types.str;
      default = null;
      example = "https://s3.example.com";
      description = "S3-compatible object storage endpoint.";
    };

    s3Bucket = mkOption {
      type = types.nullOr types.str;
      default = null;
      example = "buzz-media";
      description = "S3 bucket used for relay media.";
    };

    s3Region = mkOption {
      type = types.nullOr types.str;
      default = null;
      example = "eu-west-2";
      description = "S3 region used for relay media.";
    };

    s3AddressingStyle = mkOption {
      type = types.nullOr (
        types.enum [
          "path"
          "virtual"
        ]
      );
      default = null;
      example = "path";
      description = "S3 bucket addressing style. Null retains the relay default.";
    };

    mediaBaseUrl = mkOption {
      type = types.nullOr types.str;
      default = null;
      example = "https://buzz.example.com/media";
      description = "Public base URL for relay-hosted media.";
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
      description = "Whether NIP-OA owner attestations may grant relay membership.";
    };

    pubkeyAllowlist = mkOption {
      type = types.bool;
      default = false;
      description = "Whether pubkey-only NIP-42 authentication uses the allowlist.";
    };

    requireMediaGetAuth = mkOption {
      type = types.bool;
      default = false;
      description = "Whether media downloads require authentication.";
    };

    huddleAudioAvailable = mkOption {
      type = types.bool;
      default = true;
      description = "Whether this deployment can serve in-process huddle audio.";
    };

    auditEnabled = mkOption {
      type = types.bool;
      default = true;
      description = "Whether the relay writes its hash-chain audit log.";
    };

    autoMigrate = mkOption {
      type = types.bool;
      default = true;
      description = "Whether the relay applies embedded Postgres migrations at startup.";
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

    maxFrameBytes = mkOption {
      type = types.ints.positive;
      default = 512 * 1024;
      description = "Maximum inbound WebSocket frame size in bytes.";
    };

    slowClientGraceLimit = mkOption {
      type = types.ints.positive;
      default = 15;
      description = "Consecutive full-buffer events tolerated before disconnecting a slow client.";
    };

    corsOrigins = mkOption {
      type = types.listOf types.str;
      default = [ ];
      example = [
        "tauri://localhost"
        "https://buzz.example.com"
      ];
      description = "Allowed CORS origins. An empty list retains the permissive development default.";
    };

    ephemeralTtlOverride = mkOption {
      type = types.nullOr types.ints.positive;
      default = null;
      description = "Optional TTL override in seconds for ephemeral channels.";
    };

    git = {
      packCachePath = mkOption {
        type = types.path;
        default = "${cfg.gitRepoPath}/.pack-cache";
        defaultText = lib.literalExpression ''"''${config.services.buzz-relay.gitRepoPath}/.pack-cache"'';
        description = "Directory used for cached git pack files.";
      };

      maxPackBytes = mkOption {
        type = types.ints.positive;
        default = 500 * 1024 * 1024;
        description = "Maximum accepted git pack size in bytes.";
      };

      maxRepoBytes = mkOption {
        type = types.ints.positive;
        default = cfg.git.maxPackBytes * 2;
        defaultText = lib.literalExpression "config.services.buzz-relay.git.maxPackBytes * 2";
        description = "Maximum stored git repository size in bytes.";
      };

      packCacheMaxBytes = mkOption {
        type = types.ints.positive;
        default = cfg.git.maxRepoBytes * 5;
        defaultText = lib.literalExpression "config.services.buzz-relay.git.maxRepoBytes * 5";
        description = "Maximum aggregate git pack cache size in bytes.";
      };

      packCacheMaxConcurrentPopulations = mkOption {
        type = types.ints.positive;
        default = 2;
        description = "Maximum number of concurrent git pack cache populations.";
      };

      maxReposPerPubkey = mkOption {
        type = types.ints.positive;
        default = 100;
        description = "Maximum number of git repositories per pubkey.";
      };

      maxConcurrentOps = mkOption {
        type = types.ints.positive;
        default = 20;
        description = "Maximum number of concurrent git subprocesses.";
      };
    };

    logFilter = mkOption {
      type = types.str;
      default = "buzz_relay=info";
      description = "RUST_LOG filter for the relay.";
    };

    environment = mkOption {
      type = types.attrsOf (
        types.oneOf [
          types.str
          types.int
          types.bool
          types.path
        ]
      );
      default = { };
      example = {
        BUZZ_RELAY_PRIVATE_KEY = "32-byte-hex-private-key";
      };
      description = ''
        Additional relay environment variables. Values here are copied into the
        Nix store; use environmentFile for private keys, passwords, and access
        credentials.
      '';
    };

    environmentFile = mkOption {
      type = types.nullOr (types.either types.str (types.listOf types.str));
      default = null;
      example = "/run/secrets/buzz-relay.env";
      description = "Runtime environment file or files containing secrets and deployment overrides.";
    };

    path = mkOption {
      type = types.listOf types.package;
      default = with pkgs; [
        curl
        git
        openssl
      ];
      defaultText = lib.literalExpression "with pkgs; [ curl git openssl ]";
      description = "Packages available to relay-managed git subprocesses and hooks.";
    };

    extraReadWritePaths = mkOption {
      type = types.listOf types.path;
      default = [ ];
      description = "Additional paths the hardened systemd unit may write to.";
    };

    openFirewall = mkOption {
      type = types.bool;
      default = false;
      description = "Whether to open the public relay port in the NixOS firewall.";
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
      "d ${cfg.git.packCachePath} 0750 ${cfg.user} ${cfg.group} - -"
    ];

    systemd.services.buzz-relay = {
      description = "Buzz relay";
      wantedBy = [ "multi-user.target" ];
      wants = [ "network-online.target" ];
      after = [ "network-online.target" ];

      path = cfg.path;
      environment = builtins.mapAttrs (_: envValue) (relayEnvironment // cfg.environment);

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
        ReadWritePaths = [
          cfg.dataDir
          cfg.gitRepoPath
          cfg.git.packCachePath
        ]
        ++ cfg.extraReadWritePaths;
      }
      // optionalAttrs (cfg.environmentFile != null) {
        EnvironmentFile = cfg.environmentFile;
      };
    };

    networking.firewall.allowedTCPPorts = lib.optional cfg.openFirewall cfg.port;
  };
}
