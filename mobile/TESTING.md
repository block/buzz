# Mobile Testing

## Reproducible Android emulator

On x86-64 Linux, use the repository's dedicated Nix shell. It pins the Android
36 SDK and default x86-64 system image together with the build-tools, emulator,
NDK, CMake, Java, and Flutter versions used by the project:

```bash
nix develop .#mobile-android
just mobile-emulator start
just mobile-emulator status
```

The AVD is named and configured by `scripts/mobile-android-emulator.sh`. Its
mutable data is isolated under
`${XDG_STATE_HOME:-$HOME/.local/state}/buzz/android-emulator`, never mixed with
Android Studio's default AVDs. Override that location with
`BUZZ_ANDROID_EMULATOR_HOME` when parallel checkouts need independent state.
Use a dedicated path that does not already contain unrelated data: the helper
creates a versioned ownership marker and refuses to adopt or reset any
pre-existing unowned directory.

The default serial is `emulator-5556`. If another AVD already owns that serial,
the helper fails closed rather than reusing or stopping it. With a separate
state root, override `BUZZ_ANDROID_EMULATOR_SERIAL` for parallel devices; each
configured serial is verified against the expected Buzz AVD name before
lifecycle commands run.

The emulator runs headless by default. Use `just mobile-emulator start
--window` for interactive work. Hardware acceleration uses `/dev/kvm` when it
is available and falls back to slower software emulation otherwise.

Useful lifecycle commands:

```bash
just mobile-emulator screenshot test-results/mobile-emulator/manual.png
just mobile-emulator stop
just mobile-emulator reset  # deletes only Buzz's isolated AVD state
```

The emulator lifecycle is independent of any specific test. To run an
on-device integration target, pass its path explicitly:

```bash
just mobile-emulator-test integration_test/example_test.dart
```

Add focused specs under `mobile/integration_test/` when a feature needs real
Android rendering. Keep providers and data deterministic; use a paired debug
installation and a relay only for workflows whose integration boundary is
itself under test.
