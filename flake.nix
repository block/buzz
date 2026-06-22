{
  description = "Buzz relay NixOS module";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAllSystems = nixpkgs.lib.genAttrs systems;
    in
    {
      nixosModules = {
        buzz-relay = import ./nix/modules/buzz-relay.nix { inherit self; };
        default = self.nixosModules.buzz-relay;
      };

      packages = forAllSystems (system:
        let
          pkgs = import nixpkgs { inherit system; };
          lib = pkgs.lib;
          source = lib.cleanSourceWith {
            src = ./.;
            filter = path: type:
              let
                root = toString ./.;
                rel = lib.removePrefix "${root}/" (toString path);
                base = baseNameOf path;
              in
              !(base == ".git"
                || base == ".jj"
                || base == "target"
                || base == "node_modules"
                || lib.hasPrefix ".git/" rel
                || lib.hasPrefix ".jj/" rel
                || lib.hasPrefix "target/" rel
                || lib.hasInfix "/target/" rel
                || lib.hasInfix "/node_modules/" rel);
          };
          relayRuntime = pkgs.rustPlatform.buildRustPackage {
            pname = "buzz-relay-runtime";
            version = "0.1.0";

            src = source;
            cargoLock = {
              lockFile = ./Cargo.lock;
              allowBuiltinFetchGit = true;
            };

            cargoBuildFlags = [
              "-p"
              "buzz-relay"
              "-p"
              "buzz-admin"
            ];
            doCheck = false;

            nativeBuildInputs = with pkgs; [
              cmake
              pkg-config
            ];

            buildInputs = with pkgs; [ openssl ];

            installPhase = ''
              runHook preInstall

              mkdir -p "$out/bin"
              for bin in buzz-relay buzz-admin; do
                bin_path="$(find target -type f -path "*/release/$bin" -perm -0100 | head -n 1)"
                if [ -z "$bin_path" ]; then
                  echo "Could not find built binary: $bin" >&2
                  find target -type f -perm -0100 >&2
                  exit 1
                fi
                install -Dm755 "$bin_path" "$out/bin/$bin"
              done

              runHook postInstall
            '';

            meta = {
              description = "Buzz relay and migration binaries";
              license = lib.licenses.asl20;
              mainProgram = "buzz-relay";
            };
          };
        in
        {
          default = relayRuntime;
          buzz-runtime = relayRuntime;
          relay-runtime = relayRuntime;
        });

      checks = forAllSystems (system:
        let
          pkgs = import nixpkgs { inherit system; };
          relayModule = nixpkgs.lib.nixosSystem {
            inherit system;
            modules = [
              self.nixosModules.buzz-relay
              {
                system.stateVersion = "25.05";
                services.buzz-relay = {
                  enable = true;
                  host = "::1";
                  port = 3456;
                  autoMigrate = false;
                  openFirewall = true;
                };
              }
            ];
          };
          config = relayModule.config;
          environment = config.systemd.services.buzz-relay.environment;
          moduleCheck =
            assert environment.BUZZ_BIND_ADDR == "[::1]:3456";
            assert environment.BUZZ_AUTO_MIGRATE == "false";
            assert builtins.elem 3456 config.networking.firewall.allowedTCPPorts;
            pkgs.runCommand "buzz-relay-nixos-module-check" { } ''
              test -x ${self.packages.${system}.buzz-runtime}/bin/buzz-relay
              touch "$out"
            '';
        in
        {
          nixos-module = moduleCheck;
        });
    };
}
