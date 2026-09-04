{
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
        hash = "sha256-I7M2Fnh8yUnVsUOOl5RVD4BeIIoBTFwiRUgyB8WLvA8=";
      };
      "aarch64-darwin" = {
        urlSuffix = "osx-arm64-static-lib";
        hash = "sha256-V4Adsru3hqXTQ/UVo4/yELQBhCM4vcgE+gdTEtHNJAQ=";
      };
    };
  };

  # Vendored dependency hashes for the root and desktop Cargo.lock files.
  # Regenerate after either lock file changes by replacing its hash with
  # lib.fakeHash in nix/buzz.nix and building the affected package.
  workspaceCargoHash = "sha256-7BQWBpHdmwt9BAbDlsEmk4PIYkeRDZwYIck3kgIJolo=";
  desktopCargoHash = "sha256-ISS7c03+n2RRq4DT+gBXbS4ZHnjgjAvXnDYSvVPO4Ww=";

  # Hash for pnpm dependencies (desktop frontend).
  # Regenerate after package.json / pnpm-lock.yaml changes: remove and let nix build fetch.
  pnpmHash = "sha256-+YUfxmJOyPE5dB4vVVuArBcEliTb+sZSJoFjuPwUvx0=";
}
