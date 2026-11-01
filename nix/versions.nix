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
      "x86_64-darwin" = {
        urlSuffix = "osx-x64-static-lib";
        hash = "sha256-K9osELMaHPxF2fnhS9SYN0PsN3nTCeQtmabI+haJBD8=";
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
  workspaceCargoHash = "sha256-WJj2FSdodTpyXBLY2avrgPShtIUDiewI18tn5+AYntM=";
  desktopCargoHash = "sha256-WJj2FSdodTpyXBLY2avrgPShtIUDiewI18tn5+AYntM=";

  # Hash for pnpm dependencies (desktop frontend).
  # Regenerate after package.json / pnpm-lock.yaml changes: remove and let nix build fetch.
  pnpmHash = "sha256-o59bopq5bDr51AOLOuVYOsy7/c+i1X7zeybz5teYFA4=";
}
