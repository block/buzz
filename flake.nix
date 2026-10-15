{
  description = "Buzz sidecar binaries (buzz-acp, buzz-agent, buzz CLI) — headless/server Nix packaging, no desktop app";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };
      in
      {
        packages.default = pkgs.callPackage ./nix/buzz.nix { };
        packages.buzz-sidecars = pkgs.callPackage ./nix/buzz.nix { };
      });
}
