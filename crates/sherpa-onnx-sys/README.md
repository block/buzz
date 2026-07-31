# sherpa-onnx-sys for Buzz voice clients

This package contains the crates.io `sherpa-onnx-sys` 1.13.4 FFI sources
with a build script that supports caller-prepared mobile libraries. The
bindings, package metadata, license, and `Cargo.toml.orig` preserve their
upstream provenance.

## Link modes

`buzz-voice` exposes two mutually exclusive features:

- `static` is the default and is used by desktop and iOS clients.
- `shared` is used by Android clients.

A shared consumer must disable the default static feature:

```toml
[target.'cfg(target_os = "android")'.dependencies]
buzz-voice = { git = "https://github.com/block/buzz.git", rev = "<commit>", default-features = false, features = ["shared"] }

[target.'cfg(target_os = "ios")'.dependencies]
buzz-voice = { git = "https://github.com/block/buzz.git", rev = "<commit>", default-features = false, features = ["static"] }

[patch.crates-io]
sherpa-onnx-sys = { git = "https://github.com/block/buzz.git", rev = "<commit>" }
```

Cargo does not propagate `[patch.crates-io]` from dependencies, so the
application workspace owns that patch.

## Mobile native libraries

The application that produces the final native library owns archive download,
checksum verification, target-slice selection, and packaging.
`SHERPA_ONNX_LIB_DIR` points to the selected target's normalized link
directory:

- iOS contains `libsherpa-onnx.a` and `libonnxruntime.a`, extracted from
  the matching official XCFramework slices.
- Android contains `libsherpa-onnx-c-api.so` and `libonnxruntime.so` from
  the matching official ABI directory.

The consumer embeds these libraries and the platform C++ runtime in its Xcode
or Gradle product. Mobile builds fail closed when
`SHERPA_ONNX_LIB_DIR` is missing or invalid. Supported desktop targets select
and download the matching official archive when a caller does not provide a
library directory.

Upstream project: <https://github.com/k2-fsa/sherpa-onnx>

The upstream Apache 2.0 license is included in [LICENSE](LICENSE).
