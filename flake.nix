{
  description = "Buzz desktop app, relay server, and agent tools";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
      fenix,
    }:
    flake-utils.lib.eachSystem
      [
        "x86_64-linux"
        "aarch64-linux"
        "aarch64-darwin"
      ]
      (
        system:
        let
          pkgs = import nixpkgs { inherit system; };
          toolchain = fenix.packages.${system}.fromToolchainFile {
            file = ./rust-toolchain.toml;
            sha256 = "sha256-gh/xTkxKHL4eiRXzWv8KP7vfjSk61Iq48x47BEDFgfk=";
          };
          rustPlatform = pkgs.makeRustPlatform {
            cargo = toolchain;
            rustc = toolchain;
          };
          source = pkgs.lib.cleanSourceWith {
            src = self;
            filter =
              path: type:
              let
                name = baseNameOf path;
              in
              !(
                (
                  type == "directory"
                  && builtins.elem name [
                    ".git"
                    ".github"
                    "deploy"
                    "docs"
                    "mobile"
                    "nix"
                    "result"
                  ]
                )
                || builtins.elem name [
                  "flake.lock"
                  "flake.nix"
                  "README.md"
                ]
              );
          };
          buzzPackages = pkgs.callPackage ./nix/buzz.nix {
            src = source;
            inherit rustPlatform;
          };
        in
        {
          packages = {
            inherit (buzzPackages) buzz-desktop buzz-sidecars;
            default = buzzPackages.buzz-desktop;
          }
          // pkgs.lib.optionalAttrs pkgs.stdenv.hostPlatform.isLinux {
            inherit (buzzPackages) buzz-relay;
          };

          apps = pkgs.lib.optionalAttrs pkgs.stdenv.hostPlatform.isLinux {
            buzz-relay = {
              type = "app";
              program = "${buzzPackages.buzz-relay}/bin/buzz-relay";
              meta.description = "Run the Buzz relay server";
            };
            buzz-admin = {
              type = "app";
              program = "${buzzPackages.buzz-relay}/bin/buzz-admin";
              meta.description = "Run the Buzz relay administration CLI";
            };
          };

          checks = pkgs.lib.optionalAttrs pkgs.stdenv.hostPlatform.isLinux {
            nixos-module =
              let
                relaySystem = nixpkgs.lib.nixosSystem {
                  inherit system;
                  modules = [
                    self.nixosModules.buzz-relay
                    {
                      system.stateVersion = "25.05";
                      services.buzz-relay = {
                        enable = true;
                        host = "::1";
                        port = 3456;
                        healthPort = 3457;
                        metricsPort = 3458;
                        autoMigrate = false;
                        openFirewall = true;
                        readDatabasePoolSize = 24;
                        replicaReadMaxAgeMs = 1000;
                        s3AddressingStyle = "virtual";
                        environmentFile = [ "-/run/secrets/buzz-relay" ];
                        environment.BUZZ_RATE_LIMIT_HUMAN_MESSAGES_PER_MIN = 120;
                      };
                    }
                  ];
                };
                config = relaySystem.config;
                environment = config.systemd.services.buzz-relay.environment;
              in
              assert environment.BUZZ_BIND_ADDR == "[::1]:3456";
              assert environment.BUZZ_AUTO_MIGRATE == "false";
              assert environment.BUZZ_DB_READ_POOL_SIZE == "24";
              assert environment.BUZZ_REPLICA_READ_MAX_AGE_MS == "1000";
              assert environment.BUZZ_S3_ADDRESSING_STYLE == "virtual";
              assert environment.BUZZ_RATE_LIMIT_HUMAN_MESSAGES_PER_MIN == "120";
              assert
                config.systemd.services.buzz-relay.serviceConfig.EnvironmentFile == [
                  "-/run/secrets/buzz-relay"
                ];
              assert builtins.elem 3456 config.networking.firewall.allowedTCPPorts;
              pkgs.runCommand "buzz-relay-nixos-module-check" { } ''
                test -x ${buzzPackages.buzz-relay}/bin/buzz-relay
                test -x ${buzzPackages.buzz-relay}/bin/buzz-admin
                touch "$out"
              '';
          };

          formatter = pkgs.nixfmt;

          devShells.default = pkgs.mkShell {
            inputsFrom = [ buzzPackages.buzz-desktop ];
            packages = [
              toolchain
              pkgs.just
            ];
            SHERPA_ONNX_ARCHIVE_DIR = buzzPackages.buzz-desktop.passthru.sherpaOnnxArchiveDir;
          };
        }
      )
    // {
      nixosModules = {
        buzz-relay = import ./nix/modules/buzz-relay.nix { inherit self; };
        default = self.nixosModules.buzz-relay;
      };
    };
}
