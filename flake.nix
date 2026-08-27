{
  description = "Buzz — relay, desktop app, and CLI for the Buzz messaging platform";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    # x86_64-darwin was dropped from nixos-unstable (26.11). Pin to a commit
    # from 2026-05-31 (before the drop) that still supports Intel Darwin and
    # has Rust 1.85+ (edition 2024). Nixpkgs 26.05 will be the last release
    # to support x86_64-darwin.
    nixpkgs-darwin.url = "github:NixOS/nixpkgs/5f85796ab70f9a6ac935b366065d4565288947ac";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs =
    { self, nixpkgs, nixpkgs-darwin, flake-utils, ... }:
    let
      # Desktop release version — bump on every desktop release.
      # Keep in sync with the latest desktop-v* tag on block/buzz.
      desktopVersion = "0.5.20";

      # All standard Nix target systems. The relay and CLI are cross-platform
      # Rust; the desktop source build covers all four, the prebuilt covers a
      # subset (see desktopAssets below).
      allSystems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];

      # Prebuilt desktop release assets. aarch64-linux has no prebuilt desktop
      # binary — the project does not ship one. Use buzz-desktop (source build)
      # for aarch64-linux instead.
      desktopAssets = {
        "aarch64-darwin" = {
          file = "Buzz_${desktopVersion}_aarch64.app.tar.gz";
          sha256 = "sha256-WGViFkd1aiC4ur5mDDT3a0cNvaobCeA0jNLzD54fSx0=";
        };
        "x86_64-darwin" = {
          file = "Buzz_${desktopVersion}_x64.app.tar.gz";
          sha256 = "sha256-aF515sREKGWpiW01/IWy4KO1Wq6rQHj/GRm5X0UoC1U=";
        };
        "x86_64-linux" = {
          file = "Buzz_${desktopVersion}_amd64.AppImage";
          sha256 = "sha256-j6qvBOygHA5sFeadNzSCd1R3pub3d6nrmOof6d6Ut84=";
        };
      };

      # Git dependency output hashes for Cargo.lock.
      # Regenerate after Cargo.lock changes: remove one, nix build will error
      # with the correct hash.
      cargoOutputHashes = {
        "aws-creds-0.39.1" = "sha256-QAAm1phmeLFtDRgfDCoHijN1ce/rYzh18KziOUbL+hw=";
        "mesh-llm-api-client-0.75.1" = "sha256-RXjmM66u40cxnacbvTtCFJShMK4BM+MHOyJ2vQ7Gw60=";
      };

      forAllSystems = f: nixpkgs.lib.genAttrs allSystems (system: f system);

      # Select the right nixpkgs for the system.
      # x86_64-darwin uses the pinned pre-drop revision (unstable dropped it).
      pkgsFor =
        system:
        if system == "x86_64-darwin" then
          nixpkgs-darwin.legacyPackages.${system}
        else
          nixpkgs.legacyPackages.${system};

      # Shared build configuration for Rust workspace members.
      mkRustPackage =
        {
          pname,
          cargoPackage,
          pkgVersion,
          metaDescription,
          mainProgram,
        }:
        system:
        let
          pkgs = pkgsFor system;
        in
        pkgs.rustPlatform.buildRustPackage {
          inherit pname;
          version = pkgVersion;
          src = pkgs.lib.cleanSource ./.;
          cargoBuildFlags = [ "-p" cargoPackage ];
          cargoLock = {
            lockFile = ./Cargo.lock;
            outputHashes = cargoOutputHashes;
          };
          nativeBuildInputs = [ pkgs.pkg-config ];
          buildInputs =
            pkgs.lib.optionals pkgs.stdenv.isLinux [ pkgs.openssl ];
          doCheck = false;
          meta = with pkgs.lib; {
            description = metaDescription;
            homepage = "https://github.com/block/buzz";
            license = licenses.asl20;
            inherit mainProgram;
            platforms = allSystems;
          };
        };

      # Source-built desktop app (cargo-tauri + pnpm + native deps).
      # This is the default buzz-desktop package.
      desktopFromSource =
        system:
        let
          pkgs = pkgsFor system;
        in
        pkgs.callPackage ./nix/buzz.nix {
          src = pkgs.lib.cleanSource ./.;
        };

      # Prebuilt desktop app wrapper (release artifacts).
      # Faster to install than the source build, but not available on all
      # platforms and cannot be patched.
      desktopPrebuilt =
        system:
        let
          pkgs = pkgsFor system;
          asset = desktopAssets.${system};
        in
        pkgs.stdenv.mkDerivation {
          pname = "buzz-desktop-prebuilt";
          version = desktopVersion;
          src = pkgs.fetchurl {
            url = "https://github.com/block/buzz/releases/download/desktop-v${desktopVersion}/${asset.file}";
            sha256 = asset.sha256;
          };
          sourceRoot = ".";
          nativeBuildInputs =
            pkgs.lib.optionals pkgs.stdenv.isLinux [ pkgs.autoPatchelfHook ];
          buildInputs =
            pkgs.lib.optionals pkgs.stdenv.isLinux [ pkgs.stdenv.cc.cc.lib ];
          dontConfigure = true;
          dontBuild = true;
          installPhase =
            if pkgs.stdenv.isDarwin then
              ''
                runHook preInstall
                mkdir -p "$out/Applications"
                tar xzf "$src" -C "$out/Applications"
                runHook postInstall
              ''
            else
              ''
                runHook preInstall
                mkdir -p "$out/bin"
                cp "$src" "$out/bin/buzz-desktop"
                chmod +x "$out/bin/buzz-desktop"
                runHook postInstall
              '';
          meta = with pkgs.lib; {
            description = "Buzz desktop app (prebuilt release binary) — Tauri 2 + React 19 desktop client";
            homepage = "https://github.com/block/buzz";
            downloadPage = "https://github.com/block/buzz/releases";
            license = licenses.asl20;
            mainProgram = "buzz-desktop";
            platforms = builtins.attrNames desktopAssets;
            sourceProvenance = [ sourceTypes.binaryNativeCode ];
          };
        };
    in
    {
      packages = forAllSystems (
        system:
        let
          buzz-cli = mkRustPackage {
            pname = "buzz-cli";
            cargoPackage = "buzz-cli";
            pkgVersion = "0.1.0";
            metaDescription = "Buzz CLI — agent-first command line interface";
            mainProgram = "buzz";
          } system;
          buzz-relay = mkRustPackage {
            pname = "buzz-relay";
            cargoPackage = "buzz-relay";
            pkgVersion = "0.2.1";
            metaDescription = "Buzz relay — WebSocket relay server";
            mainProgram = "buzz-relay";
          } system;
        in
        rec {
          inherit buzz-cli buzz-relay;
          buzz = buzz-cli;
          # Source-built desktop (default) — builds from the Tauri + React
          # source with cargo-tauri, pnpm, and native dependencies.
          buzz-desktop = desktopFromSource system;
          # Prebuilt desktop (alternative) — wraps the release artifact for
          # faster installation where a prebuilt binary exists.
          buzz-desktop-prebuilt =
            if desktopAssets ? ${system} then
              desktopPrebuilt system
            else
              throw "buzz-desktop-prebuilt: no release binary for ${system}";
          default = buzz-cli;
        }
      );

      apps = forAllSystems (
        system:
        {
          buzz = {
            type = "app";
            program = "${self.packages.${system}.buzz-cli}/bin/buzz";
          };
          buzz-relay = {
            type = "app";
            program = "${self.packages.${system}.buzz-relay}/bin/buzz-relay";
          };
          default = self.apps.${system}.buzz;
        }
        // nixpkgs.lib.optionalAttrs (desktopAssets ? ${system}) {
          buzz-desktop-prebuilt = {
            type = "app";
            program =
              if system == "x86_64-linux" then
                "${self.packages.${system}.buzz-desktop-prebuilt}/bin/buzz-desktop"
              else
                "${self.packages.${system}.buzz-desktop-prebuilt}/Applications/Buzz.app/Contents/MacOS/Buzz";
          };
        }
      );

      checks = forAllSystems (
        system:
        {
          buzz-cli = self.packages.${system}.buzz-cli;
          buzz-relay = self.packages.${system}.buzz-relay;
        }
        // nixpkgs.lib.optionalAttrs (desktopAssets ? ${system}) {
          buzz-desktop-prebuilt = self.packages.${system}.buzz-desktop-prebuilt;
        }
      );

      devShells = forAllSystems (
        system:
        let
          pkgs = pkgsFor system;
        in
        {
          default = pkgs.mkShell {
            nativeBuildInputs = [ pkgs.pkg-config ];
            buildInputs =
              [ pkgs.cargo pkgs.rustc ]
              ++ pkgs.lib.optionals pkgs.stdenv.isLinux [ pkgs.openssl ]
              # Runtime service deps for buzz-relay development
              ++ [ pkgs.postgresql pkgs.redis ];
          };
        }
      );
    };
}
