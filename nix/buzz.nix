{
  lib,
  rustPlatform,
  fetchPnpmDeps,
  fetchurl,
  linkFarm,
  runCommand,
  symlinkJoin,
  perl,
  cmake,
  cargo-tauri,
  git,
  nodejs,
  pnpm,
  pnpmConfigHook,
  pkg-config,
  autoPatchelfHook,
  wrapGAppsHook3,
  gst_all_1,
  glib-networking,
  bzip2,
  openssl,
  webkitgtk_4_1,
  alsa-lib,
  libopus,
  dbus,
  glib,
  gtk3,
  libayatana-appindicator,
  libsoup_3,
  librsvg,
  libiconv,
  stdenv,
  src,
}:

let
  versions = import ./versions.nix;
  desktopVersion =
    (builtins.fromTOML (builtins.readFile (src + "/desktop/src-tauri/Cargo.toml"))).package.version;
  relayVersion =
    (builtins.fromTOML (builtins.readFile (src + "/crates/buzz-relay/Cargo.toml"))).package.version;

  sherpaOnnxSystemConfig =
    versions.sherpaOnnx.systems.${stdenv.hostPlatform.system}
      or (builtins.throw "Unsupported platform: ${stdenv.hostPlatform.system}. Supported platforms: ${builtins.toString (builtins.attrNames versions.sherpaOnnx.systems)}");

  sherpaOnnxArchive = fetchurl {
    name = "sherpa-onnx-v${versions.sherpaOnnx.version}-${sherpaOnnxSystemConfig.urlSuffix}.tar.bz2";
    url = "https://github.com/k2-fsa/sherpa-onnx/releases/download/v${versions.sherpaOnnx.version}/sherpa-onnx-v${versions.sherpaOnnx.version}-${sherpaOnnxSystemConfig.urlSuffix}.tar.bz2";
    hash = sherpaOnnxSystemConfig.hash;
  };

  sherpaOnnxArchiveDir = linkFarm "sherpa-onnx-archive" [
    {
      name = "sherpa-onnx-v${versions.sherpaOnnx.version}-${sherpaOnnxSystemConfig.urlSuffix}.tar.bz2";
      path = sherpaOnnxArchive;
    }
  ];

  workspaceCargoDeps = rustPlatform.fetchCargoVendor {
    inherit src;
    name = "buzz-sidecars-${desktopVersion}";
    hash = versions.workspaceCargoHash;
  };

  sidecarPackages = [
    "buzz-acp"
    "buzz-agent"
    "buzz-backend-kubernetes"
    "buzz-dev-mcp"
    "git-credential-nostr"
    "buzz-cli"
  ];
  sidecarBinNames = [
    "buzz-acp"
    "buzz-agent"
    "buzz-backend-kubernetes"
    "buzz-dev-mcp"
    "git-credential-nostr"
    "buzz"
  ];
  sidecars = rustPlatform.buildRustPackage {
    pname = "buzz-sidecars";
    version = desktopVersion;

    inherit src;

    cargoDeps = workspaceCargoDeps;

    cargoBuildFlags = lib.concatLists (
      map (p: [
        "-p"
        p
      ]) sidecarPackages
    );
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
      license = licenses.asl20;
      platforms = [
        "x86_64-linux"
        "aarch64-linux"
        "aarch64-darwin"
      ];
    };
  };

  gitClientSidecars = rustPlatform.buildRustPackage {
    pname = "buzz-git-client-sidecars";
    version = desktopVersion;

    inherit src;

    cargoDeps = workspaceCargoDeps;
    cargoBuildFlags = [
      "-p"
      "buzz-cli"
      "-p"
      "git-credential-nostr"
    ];
    doCheck = false;

    nativeBuildInputs = [ pkg-config ];
    buildInputs = lib.optionals stdenv.hostPlatform.isLinux [ openssl ];
  };

  mkSidecarBinary =
    {
      pname,
      binaryName,
      description,
    }:
    runCommand "${pname}-${desktopVersion}"
      {
        meta = with lib; {
          inherit description;
          homepage = "https://github.com/block/buzz";
          license = licenses.asl20;
          mainProgram = binaryName;
          platforms = [
            "x86_64-linux"
            "aarch64-linux"
            "aarch64-darwin"
          ];
        };
      }
      ''
        mkdir -p "$out/bin"
        ln -s ${gitClientSidecars}/bin/${binaryName} "$out/bin/${binaryName}"
      '';

  buzzCli = mkSidecarBinary {
    pname = "buzz-cli";
    binaryName = "buzz";
    description = "Command-line client for Buzz relays";
  };

  gitCredentialNostr = mkSidecarBinary {
    pname = "git-credential-nostr";
    binaryName = "git-credential-nostr";
    description = "NIP-98 credential helper for Buzz-hosted git repositories";
  };

  buzzGitTools = symlinkJoin {
    name = "buzz-git-tools-${desktopVersion}";
    paths = [
      buzzCli
      gitCredentialNostr
      git
    ];
    meta = with lib; {
      description = "Buzz CLI, Nostr git credential helper, and git client";
      homepage = "https://github.com/block/buzz";
      license = [
        licenses.asl20
        licenses.gpl2Only
      ];
      platforms = [
        "x86_64-linux"
        "aarch64-linux"
        "aarch64-darwin"
      ];
    };
  };

  relayRuntime = rustPlatform.buildRustPackage {
    pname = "buzz-relay-runtime";
    version = relayVersion;

    inherit src;

    cargoHash = versions.workspaceCargoHash;

    cargoBuildFlags = [
      "-p"
      "buzz-relay"
      "-p"
      "buzz-admin"
    ];
    doCheck = false;

    nativeBuildInputs = [
      cmake
      pkg-config
    ];

    buildInputs = lib.optionals stdenv.hostPlatform.isLinux [
      openssl
    ];

    meta = with lib; {
      description = "Buzz relay server and administration CLI";
      homepage = "https://github.com/block/buzz";
      license = licenses.asl20;
      platforms = platforms.linux;
      mainProgram = "buzz-relay";
    };
  };
in
{
  buzz-relay = relayRuntime;
  buzz-sidecars = sidecars;
  buzz-cli = buzzCli;
  git-credential-nostr = gitCredentialNostr;
  buzz-git-tools = buzzGitTools;

  buzz-desktop = rustPlatform.buildRustPackage (finalAttrs: {
    pname = "buzz-desktop";
    version = desktopVersion;
    inherit src;

    cargoRoot = "desktop/src-tauri";
    buildAndTestSubdir = "desktop/src-tauri";

    cargoHash = versions.desktopCargoHash;

    doCheck = false;

    AWS_LC_SYS_CMAKE_BUILDER = 1;

    pnpmDeps = fetchPnpmDeps {
      pname = "buzz-workspace";
      inherit (finalAttrs) version src;
      inherit pnpm;
      fetcherVersion = 4;
      hash = versions.pnpmHash;
    };

    pnpmWorkspaces = [ "buzz" ];

    preBuild = ''
      export SHERPA_ONNX_ARCHIVE_DIR=${sherpaOnnxArchiveDir}
      ${lib.optionalString stdenv.hostPlatform.isLinux ''
        export LD_LIBRARY_PATH=${lib.makeLibraryPath [ bzip2 ]}
      ''}

      mkdir -p desktop/src-tauri/binaries
      for bin in ${builtins.concatStringsSep " " sidecarBinNames}; do
        cp ${sidecars}/bin/$bin desktop/src-tauri/binaries/$bin-${stdenv.hostPlatform.config}
      done
    '';

    # Project clone/fetch/push operations are performed by git itself. The
    # Nostr credential helper is bundled as a Tauri sidecar, while this makes
    # a compatible git client available to the launched desktop application.
    preFixup = lib.optionalString stdenv.hostPlatform.isLinux ''
      gappsWrapperArgs+=(--prefix PATH : "${lib.makeBinPath [ git ]}")
    '';

    nativeBuildInputs = [
      cmake
      cargo-tauri.hook
      nodejs
      perl
      pnpmConfigHook
      pnpm
      pkg-config
    ]
    ++ lib.optionals stdenv.hostPlatform.isLinux [
      autoPatchelfHook
      wrapGAppsHook3
    ];

    buildInputs =
      lib.optionals stdenv.hostPlatform.isLinux [
        alsa-lib
        bzip2
        libopus
        dbus
        glib
        gtk3
        libayatana-appindicator
        libsoup_3
        librsvg
        glib-networking
        openssl
        webkitgtk_4_1
        gst_all_1.gstreamer
        gst_all_1.gst-plugins-base
        gst_all_1.gst-plugins-good
        gst_all_1.gst-plugins-bad
      ]
      ++ lib.optionals stdenv.hostPlatform.isDarwin [ libiconv ];

    passthru = {
      inherit sherpaOnnxArchiveDir sidecars;
    };

    meta = with lib; {
      description = "Buzz desktop app";
      homepage = "https://buzz.ai";
      # sherpa-onnx statically links GPL-3.0-or-later eSpeak NG code.
      license = licenses.gpl3Plus;
      sourceProvenance = [ sourceTypes.binaryNativeCode ];
      platforms = [
        "x86_64-linux"
        "aarch64-linux"
        "aarch64-darwin"
      ];
      mainProgram = "buzz-desktop";
    };
  });
}
