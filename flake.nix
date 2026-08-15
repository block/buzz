{
  description = "Buzz desktop development environment";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

    # Nixpkgs 26.11 dropped x86_64-darwin while Buzz still ships Intel
    # macOS builds. Keep that system on the supported 26.05 branch.
    nixpkgs-darwin.url = "github:NixOS/nixpkgs/nixpkgs-26.05-darwin";

    flake-parts.url = "github:hercules-ci/flake-parts";

    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    inputs@{
      flake-parts,
      nixpkgs,
      nixpkgs-darwin,
      rust-overlay,
      ...
    }:
    flake-parts.lib.mkFlake { inherit inputs; } {
      systems = [
        "aarch64-darwin"
        "aarch64-linux"
        "x86_64-darwin"
        "x86_64-linux"
      ];

      perSystem =
        { system, ... }:
        let
          nixpkgsForSystem = if system == "x86_64-darwin" then nixpkgs-darwin else nixpkgs;

          pkgs = import nixpkgsForSystem {
            inherit system;
            overlays = [ (import rust-overlay) ];
          };

          toolchainChannel = (builtins.fromTOML (builtins.readFile ./rust-toolchain.toml)).toolchain.channel;

          rustToolchain = pkgs.rust-bin.stable.${toolchainChannel}.default;

          commonPackages = with pkgs; [
            cargo-deny
            cargo-nextest
            cmake
            curl
            file
            git
            just
            lefthook
            ninja
            nodejs_24
            openssl
            perl
            pkg-config
            pnpm_11
            python3
            rust-analyzer
            rustToolchain
            wget
          ];

          linuxLibraries = with pkgs; [
            alsa-lib
            atk
            cairo
            gdk-pixbuf
            glib
            gtk3
            libayatana-appindicator
            libopus
            librsvg
            openssl
            webkitgtk_4_1
            xdotool
          ];

          linuxPackages =
            with pkgs;
            [
              gcc
              mold
              patchelf
            ]
            ++ linuxLibraries;
        in
        {
          formatter = pkgs.nixfmt;

          devShells.default = pkgs.mkShell {
            packages = commonPackages ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux linuxPackages;

            CMAKE_POLICY_VERSION_MINIMUM = "3.5";
            LD_LIBRARY_PATH = pkgs.lib.optionalString pkgs.stdenv.hostPlatform.isLinux (
              pkgs.lib.makeLibraryPath linuxLibraries
            );

            shellHook = ''
              export PATH="$PWD/node_modules/.bin:$PATH"

              echo "Buzz development shell"
              echo "  Rust: $(rustc --version)"
              echo "  Node: $(node --version)"
              echo "  pnpm: $(pnpm --version)"
            '';
          };
        };
    };
}
