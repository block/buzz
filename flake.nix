{
  description = "Buzz desktop app and agent tools";

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
                    "nix"
                    "result"
                  ]
                )
                || builtins.elem name [
                  "flake.lock"
                  "flake.nix"
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
          };

          devShells.default = pkgs.mkShell {
            inputsFrom = [ buzzPackages.buzz-desktop ];
            packages = [
              toolchain
              pkgs.just
            ];
            SHERPA_ONNX_ARCHIVE_DIR = buzzPackages.buzz-desktop.passthru.sherpaOnnxArchiveDir;
          };
        }
      );
}
