{
  lib,
  rustPlatform,
  fetchPnpmDeps,
  fetchurl,
  cmake,
  cargo-tauri,
  nodejs,
  pnpm,
  pnpmConfigHook,
  pkg-config,
  wrapGAppsHook4,
  makeWrapper,
  gst_all_1,
  glib-networking,
  openssl,
  webkitgtk_4_1,
  alsa-lib,
  libopus,
  dbus,
  glib,
  gtk3,
  libsoup_3,
  librsvg,
  darwin,
  libiconv,
  stdenv,
  src,
}:

let
  versions = import ./versions.nix;

  sherpaOnnxSystemConfig = versions.sherpaOnnx.systems.${stdenv.hostPlatform.system} or (builtins.throw "Unsupported platform: ${stdenv.hostPlatform.system}. Supported platforms: ${builtins.toString (builtins.attrNames versions.sherpaOnnx.systems)}");

  sherpaOnnxArchive = fetchurl {
    name = "sherpa-onnx-v${versions.sherpaOnnx.version}-${sherpaOnnxSystemConfig.urlSuffix}.tar.bz2";
    url = "https://github.com/k2-fsa/sherpa-onnx/releases/download/v${versions.sherpaOnnx.version}/sherpa-onnx-v${versions.sherpaOnnx.version}-${sherpaOnnxSystemConfig.urlSuffix}.tar.bz2";
    hash = sherpaOnnxSystemConfig.hash;
  };

  sidecarPackages = [ "buzz-acp" "buzz-agent" "buzz-dev-mcp" "git-credential-nostr" "buzz-cli" ];
  sidecarBinNames = [ "buzz-acp" "buzz-agent" "buzz-dev-mcp" "git-credential-nostr" "buzz" ];

  sidecars = rustPlatform.buildRustPackage {
    pname = "buzz-sidecars";
    version = versions.buzzVersion;

    src = src;

    cargoLock = {
      lockFileContents = builtins.readFile (src + "/Cargo.lock");
      outputHashes = versions.sidecarCargoOutputHashes;
    };

    cargoBuildFlags = lib.concatLists (map (p: [ "-p" p ]) sidecarPackages);
    doCheck = false;

    nativeBuildInputs = [
      cmake
      pkg-config
    ];

    buildInputs = lib.optionals stdenv.hostPlatform.isLinux [
      openssl
    ];

    meta = with lib; {
      description = "Buzz sidecar binaries";
      license = licenses.unfree;
      platforms = [ "x86_64-linux" "aarch64-linux" "aarch64-darwin" ];
    };
  };
in
rustPlatform.buildRustPackage (finalAttrs: {
  pname = "buzz-desktop";
  version = versions.buzzVersion;

  src = src;

  cargoRoot = "desktop/src-tauri";
  buildAndTestSubdir = "desktop/src-tauri";

  cargoLock = {
    lockFileContents = builtins.readFile (src + "/desktop/src-tauri/Cargo.lock");
    outputHashes = versions.desktopCargoOutputHashes;
  };

  doCheck = false;

  pnpmDeps = fetchPnpmDeps {
    inherit (finalAttrs) pname version src;
    pnpm = pnpm;
    fetcherVersion = 4;
    hash = versions.pnpmHash;
  };

  preBuild = ''
    mkdir -p $TMPDIR/sherpa-onnx-archive
    ln -sf ${sherpaOnnxArchive} $TMPDIR/sherpa-onnx-archive/sherpa-onnx-v${versions.sherpaOnnx.version}-${sherpaOnnxSystemConfig.urlSuffix}.tar.bz2
    export SHERPA_ONNX_ARCHIVE_DIR=$TMPDIR/sherpa-onnx-archive

    mkdir -p desktop/src-tauri/binaries
    for bin in ${builtins.concatStringsSep " " sidecarBinNames}; do
      cp ${sidecars}/bin/$bin desktop/src-tauri/binaries/$bin-${stdenv.hostPlatform.config}
    done
  '';

  nativeBuildInputs = [
    cmake
    cargo-tauri.hook
    nodejs
    pnpmConfigHook
    pnpm
    pkg-config
  ] ++ lib.optionals stdenv.hostPlatform.isLinux [ wrapGAppsHook4 ];

  buildInputs = lib.optionals stdenv.hostPlatform.isLinux [
    alsa-lib
    libopus
    dbus
    glib
    gtk3
    libsoup_3
    librsvg
    glib-networking
    openssl
    webkitgtk_4_1
    gst_all_1.gstreamer
    gst_all_1.gst-plugins-base
    gst_all_1.gst-plugins-good
    gst_all_1.gst-plugins-bad
  ] ++ lib.optionals stdenv.hostPlatform.isDarwin [
    libiconv
    darwin.apple_sdk.frameworks.CoreFoundation
    darwin.apple_sdk.frameworks.Security
    darwin.apple_sdk.frameworks.SystemConfiguration
    darwin.apple_sdk.frameworks.AppKit
  ];

  meta = with lib; {
    description = "Buzz desktop app";
    homepage = "https://buzz.ai";
    license = licenses.unfree;
      platforms = [ "x86_64-linux" "aarch64-linux" "aarch64-darwin" ];
    mainProgram = "buzz-desktop";
  };
})
