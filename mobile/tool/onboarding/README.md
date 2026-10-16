# Onboarding interoperability check

Use the production entry point `lib/main.dart` for onboarding walkthroughs.
The temporary preview screens and identity-pause mode have been removed.

`hpke_interop.swift`, compiled with `ios/Runner/HostedHpkeBackup.swift`, checks
native sealing against the existing synthetic fixture at
`../desktop/src-tauri/src/testdata/hpke_nsec_backup_v1.json` (relative to `mobile/`).
The fixture keys are test data and must never be used for a real identity.
