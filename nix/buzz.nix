{ lib, rustPlatform, fetchFromGitHub, pkg-config, openssl }:

let
  versions = import ./versions.nix;

  src = fetchFromGitHub {
    owner = "block";
    repo = "buzz";
    rev = versions.buzzRev;
    hash = versions.buzzSrcHash;
  };

  # Only the CLI-side binaries a headless/server deployment needs. The
  # desktop app (Tauri + pnpm frontend + sherpa-onnx) and its two
  # desktop-only sidecars (buzz-dev-mcp, git-credential-nostr) are out of
  # scope here — see flake.nix's description and the PR this shipped in.
  sidecarPackages = [ "buzz-acp" "buzz-agent" "buzz-cli" ];
in
rustPlatform.buildRustPackage {
  pname = "buzz-sidecars";
  version = builtins.substring 0 8 versions.buzzRev;
  inherit src;

  cargoLock = {
    lockFileContents = builtins.readFile (src + "/Cargo.lock");
    outputHashes = versions.sidecarCargoOutputHashes;
  };

  cargoBuildFlags = lib.concatMap (p: [ "-p" p ]) sidecarPackages;
  # The workspace test suite covers crates this build doesn't compile (the
  # relay, desktop app, etc.) and wasn't evaluated for network/workspace-wide
  # assumptions a sandboxed sidecars-only build can't satisfy.
  doCheck = false;

  nativeBuildInputs = [ pkg-config ];
  buildInputs = [ openssl ];

  meta = with lib; {
    description = "Buzz sidecar binaries (buzz-acp, buzz-agent, buzz CLI) for headless/server deployment";
    homepage = "https://github.com/block/buzz";
    license = licenses.asl20;
    platforms = [ "x86_64-linux" "aarch64-linux" "aarch64-darwin" ];
    mainProgram = "buzz-acp";
  };
}
