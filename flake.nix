{
  description = "Buzz: Nostr-native multi-agent communication and workspace platform";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs =
    {
      self,
      nixpkgs,
      rust-overlay,
      flake-utils,
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs {
          inherit system overlays;
        };
        lib = pkgs.lib;

        rustToolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;

        rustPlatform = pkgs.makeRustPlatform {
          cargo = rustToolchain;
          rustc = rustToolchain;
        };

        version = "0.1.0-unstable";

        src = lib.cleanSourceWith {
          src = ./.;
          filter =
            path: type:
            let
              baseName = baseNameOf path;
            in
            !(
              type == "directory"
              && (
                baseName == "target"
                || baseName == "node_modules"
                || baseName == ".git"
                || baseName == "dist"
              )
            )
            && !(lib.hasSuffix ".log" baseName);
        };

        cargoHash = "sha256-sihAw+rf8P2MpI3pPxg2Q8VRkZxc01U7c+wbZuml+kA=";

        mkBuzzPackage =
          {
            pname,
            crateName ? pname,
            binName ? crateName,
            description,
            doCheck ? false,
          }:
          rustPlatform.buildRustPackage {
            inherit
              pname
              version
              src
              cargoHash
              ;
            cargoBuildFlags = [
              "-p"
              crateName
            ];

            nativeBuildInputs = [ pkgs.pkg-config ];
            buildInputs =
              lib.optionals pkgs.stdenv.hostPlatform.isLinux [ pkgs.openssl ]
              ++ lib.optionals pkgs.stdenv.hostPlatform.isDarwin [
                pkgs.darwin.apple_sdk.frameworks.Security
                pkgs.darwin.apple_sdk.frameworks.SystemConfiguration
              ];

            inherit doCheck;

            meta = with lib; {
              inherit description;
              homepage = "https://github.com/block/buzz";
              license = licenses.asl20;
              mainProgram = binName;
              platforms = platforms.unix;
            };
          };

        packages = rec {
          buzz-cli = mkBuzzPackage {
            pname = "buzz-cli";
            binName = "buzz";
            description = "Agent-first CLI for the Buzz relay and workspace";
          };

          buzz-acp = mkBuzzPackage {
            pname = "buzz-acp";
            description = "Agent Control Protocol (ACP) bridge for Buzz";
          };

          buzz-agent = mkBuzzPackage {
            pname = "buzz-agent";
            description = "Minimal ACP-compliant autonomous LLM agent daemon for Buzz";
          };

          buzz-dev-mcp = mkBuzzPackage {
            pname = "buzz-dev-mcp";
            description = "Model Context Protocol (MCP) server providing dev tools for Buzz";
          };

          buzz-relay = mkBuzzPackage {
            pname = "buzz-relay";
            description = "High-performance Nostr relay server for the Buzz ecosystem";
          };

          git-credential-nostr = mkBuzzPackage {
            pname = "git-credential-nostr";
            description = "Git credential helper using Nostr keypairs and NIP-49/NIP-98 authentication";
          };

          git-sign-nostr = mkBuzzPackage {
            pname = "git-sign-nostr";
            description = "Git commit and tag signing tool using Nostr BIP-340 Schnorr signatures";
          };

          buzz-pairing-cli = mkBuzzPackage {
            pname = "buzz-pairing-cli";
            description = "CLI tool for interactive device and client pairing in Buzz";
          };

          default = buzz-cli;

          all = pkgs.symlinkJoin {
            name = "buzz-tools-${version}";
            paths = [
              buzz-cli
              buzz-acp
              buzz-agent
              buzz-dev-mcp
              buzz-relay
              git-credential-nostr
              git-sign-nostr
              buzz-pairing-cli
            ];
          };
        };

        apps = rec {
          default = buzz;
          buzz = flake-utils.lib.mkApp {
            drv = packages.buzz-cli;
            exePath = "/bin/buzz";
          };
          buzz-acp = flake-utils.lib.mkApp {
            drv = packages.buzz-acp;
            exePath = "/bin/buzz-acp";
          };
          buzz-agent = flake-utils.lib.mkApp {
            drv = packages.buzz-agent;
            exePath = "/bin/buzz-agent";
          };
          buzz-dev-mcp = flake-utils.lib.mkApp {
            drv = packages.buzz-dev-mcp;
            exePath = "/bin/buzz-dev-mcp";
          };
          buzz-relay = flake-utils.lib.mkApp {
            drv = packages.buzz-relay;
            exePath = "/bin/buzz-relay";
          };
          git-credential-nostr = flake-utils.lib.mkApp {
            drv = packages.git-credential-nostr;
            exePath = "/bin/git-credential-nostr";
          };
          git-sign-nostr = flake-utils.lib.mkApp {
            drv = packages.git-sign-nostr;
            exePath = "/bin/git-sign-nostr";
          };
        };

        devShells.default = pkgs.mkShell {
          packages =
            [
              rustToolchain
              pkgs.cargo-watch
              pkgs.cargo-edit
              pkgs.nodejs_22
              pkgs.pnpm
              pkgs.just
              pkgs.pkg-config
            ]
            ++ lib.optionals pkgs.stdenv.hostPlatform.isLinux [ pkgs.openssl ]
            ++ lib.optionals pkgs.stdenv.hostPlatform.isDarwin [
              pkgs.darwin.apple_sdk.frameworks.Security
              pkgs.darwin.apple_sdk.frameworks.SystemConfiguration
            ];

          env = {
            RUST_BACKTRACE = "1";
            OPENSSL_NO_VENDOR = "1";
          };

          shellHook = ''
            echo "🐝 Welcome to the Buzz development environment!"
            echo "Rust toolchain: $(rustc --version 2>/dev/null || echo 'cargo ready')"
            echo "Run 'cargo test -p buzz-cli' or 'just --list' to get started."
          '';
        };
      in
      {
        inherit packages apps devShells;
      }
    );
}
