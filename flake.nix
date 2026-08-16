{
  description = "Buzz desktop development environment";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

    # Nixpkgs 26.11 dropped x86_64-darwin while Buzz still ships Intel
    # macOS builds. Keep that system on the supported 26.05 branch.
    nixpkgs-darwin.url = "github:NixOS/nixpkgs/nixpkgs-26.05-darwin";

    # Buzz pins Flutter 3.41.7 through Hermit. Nixpkgs skipped that patch
    # release; 3.41.9 is the closest packaged release in the same stable line.
    nixpkgs-flutter.url = "github:NixOS/nixpkgs/7fe56c8fe4e9cee3dbe797cae9e7b74def154567";

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
      nixpkgs-flutter,
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

          flutterPkgs = import nixpkgs-flutter { inherit system; };

          toolchainChannel = (builtins.fromTOML (builtins.readFile ./rust-toolchain.toml)).toolchain.channel;

          rustToolchain = pkgs.rust-bin.stable.${toolchainChannel}.default;

          biomeTarget =
            {
              aarch64-darwin = "darwin-arm64";
              aarch64-linux = "linux-arm64";
              x86_64-darwin = "darwin-x64";
              x86_64-linux = "linux-x64";
            }
            .${system};

          biomeHash =
            {
              aarch64-darwin = "sha256-5K3KJbVulY/AuXtfU5tH060gzexeJxZglFAi9X2WbyM=";
              aarch64-linux = "sha256-rTXsRcUV7i5UyLvO2dWv6rJuEE9O01gTM3YatbF5iGs=";
              x86_64-darwin = "sha256-UlLAWG/6l5mOoErfMbSg/iK63sHoCXsWMmUWHZJ5Mzg=";
              x86_64-linux = "sha256-7HYAu+gNJXqs1uvJ1+jnspwMWHccZ9z+njWDHATSwSw=";
            }
            .${system};

          biomeTool = pkgs.stdenv.mkDerivation {
            pname = "biome";
            version = "2.4.16";
            src = pkgs.fetchurl {
              url = "https://registry.npmjs.org/@biomejs/cli-${biomeTarget}/-/cli-${biomeTarget}-2.4.16.tgz";
              hash = biomeHash;
            };
            sourceRoot = "package";
            nativeBuildInputs = pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux [
              pkgs.autoPatchelfHook
            ];
            buildInputs = pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux [
              pkgs.stdenv.cc.cc
            ];
            installPhase = ''
              runHook preInstall
              install -Dm755 biome "$out/bin/biome"
              runHook postInstall
            '';
          };

          commonPackages = with pkgs; [
            cargo-deny
            cargo-nextest
            cmake
            curl
            ffmpeg
            file
            git
            just
            jq
            lefthook
            ninja
            nodejs_24
            openssl
            perl
            pkg-config
            # pnpm honors package.json's packageManager field and dispatches to
            # the workspace-pinned release when commands run in the repo.
            pnpm_11
            python3
            rust-analyzer
            rustToolchain
            wget
          ];

          occasionalPackages = with pkgs; [
            gh
            uv
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

          gstreamerPlugins = with pkgs.gst_all_1; [
            gst-libav
            gst-plugins-bad
            gst-plugins-base
            gst-plugins-good
            gstreamer
          ];

          linuxPackages = with pkgs; [ patchelf ] ++ linuxLibraries ++ gstreamerPlugins;

          # Just runs inside the caller's current environment; it cannot switch
          # Nix shells per recipe. Keep the direnv/default shell focused on
          # desktop, web, and Rust work, and opt into the larger closures with
          # `nix develop .#mobile` or `nix develop .#full`.
          mkBuzzShell =
            {
              label,
              extraPackages ? [ ],
              withFlutter ? false,
            }:
            pkgs.mkShell (
              {
                packages =
                  commonPackages
                  ++ pkgs.lib.optionals withFlutter [ flutterPkgs.flutter ]
                  ++ extraPackages
                  ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux linuxPackages;

                CMAKE_POLICY_VERSION_MINIMUM = "3.5";
                LD_LIBRARY_PATH = pkgs.lib.optionalString pkgs.stdenv.hostPlatform.isLinux (
                  pkgs.lib.makeLibraryPath linuxLibraries
                );

                GST_PLUGIN_SYSTEM_PATH_1_0 = pkgs.lib.optionalString pkgs.stdenv.hostPlatform.isLinux (
                  pkgs.lib.makeSearchPath "lib/gstreamer-1.0" gstreamerPlugins
                );

                # The npm launcher honors this variable. Point it at Nix's patched
                # executable so Biome works on NixOS without a global nix-ld setup.
                BIOME_BINARY = "${biomeTool}/bin/biome";

                shellHook = ''
                  export PATH="$PWD/node_modules/.bin:$PATH"

                  echo "Buzz ${label} development shell"
                  echo "  Rust: $(rustc --version)"
                  echo "  Node: $(node --version)"
                  echo "  pnpm: $(pnpm --version)"
                '';
              }
              // pkgs.lib.optionalAttrs withFlutter {
                FLUTTER_SUPPRESS_ANALYTICS = "true";
              }
            );
        in
        {
          formatter = pkgs.nixfmt;

          checks.toolchain =
            pkgs.runCommand "buzz-development-toolchain-check"
              {
                nativeBuildInputs = [
                  biomeTool
                  pkgs.nodejs_24
                  pkgs.pnpm_11
                  rustToolchain
                ];
              }
              ''
                rustc --version | grep -F "rustc ${toolchainChannel} "
                node --version
                pnpm --version
                biome --version
                touch "$out"
              '';

          devShells = {
            default = mkBuzzShell { label = "desktop/web"; };
            mobile = mkBuzzShell {
              label = "mobile";
              withFlutter = true;
            };
            full = mkBuzzShell {
              label = "full";
              extraPackages = occasionalPackages;
              withFlutter = true;
            };
          };
        };
    };
}
