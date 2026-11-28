{
  # App release version — bump on every release. Keep in sync with:
  #   desktop/src-tauri/Cargo.toml, desktop/src-tauri/tauri.conf.json
  buzzVersion = "0.5.0";

  # sherpa-onnx static library archive for huddle audio.
  # Per-platform archive URL and hash.
  # Regenerate hashes after version bump: run
  #   nix-prefetch-url <url>
  # for each platform.
  sherpaOnnx = {
    version = "1.13.4";
    systems = {
      "x86_64-linux" = {
        urlSuffix = "linux-x64-static-lib";
        hash = "sha256-mLDjGZZCb254JE284ZVVSPLGTo8BxL51uFr3zaoujVw=";
      };
      "aarch64-linux" = {
        urlSuffix = "linux-aarch64-static-lib";
        hash = "sha256-03xwig2hfcj88li5qk01i8h5x00gana9g3j3n7alkjbwg0b3dcr3";
      };
      "aarch64-darwin" = {
        urlSuffix = "osx-arm64-static-lib";
        hash = "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
      };
    };
  };

  # Cargo.lock output hashes for sidecar builds (workspace root Cargo.lock).
  # Regenerate after Cargo.lock changes: remove one, nix build will error with the correct hash.
  sidecarCargoOutputHashes = {
    "aws-creds-0.39.1" = "sha256-QAAm1phmeLFtDRgfDCoHijN1ce/rYzh18KziOUbL+hw=";
    "mesh-llm-api-client-0.73.1" = "sha256-2ArkxK7Ze13mqkQB+JkuqVSCLeHpdxXHMZ0592VyEWw=";
  };

  # Cargo.lock output hashes for the desktop Tauri build (desktop/src-tauri/Cargo.lock).
  # Same package may have a different hash vs. sidecars due to different dep trees.
  desktopCargoOutputHashes = {
    "mesh-llm-api-client-0.73.1" = "sha256-OItlWwacyTtdS6LQCQDPlLmB09l4bbTX27uI8AGDQpk=";
  };

  # Hash for pnpm dependencies (desktop frontend).
  # Regenerate after package.json / pnpm-lock.yaml changes: remove and let nix build fetch.
  pnpmHash = "sha256-Tboy+MG/VvdxUpJw7Xv0oubK58MIpvChvbU30uO4M4A=";
}
