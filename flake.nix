{
  description = "Buzz desktop app";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    block-buzz = {
      url = "github:block/buzz";
      flake = false;
    };
  };

  outputs = { self, nixpkgs, flake-utils, block-buzz }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs {
          inherit system;
          config.allowUnfree = true;
        };
      in
      {
        packages.default = pkgs.callPackage ./nix/buzz.nix {
          src = block-buzz;
        };
      });
}
