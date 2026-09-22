# STRINGS MANIFEST — desktop `zh-Hans`

Tracking sheet for every key shipped in `en.json` / `zh-Hans.json`
(spec §AC-011). One row per key; extend as each PR extracts more surfaces.
A key may not be merged without a row here.

Status values: `in-pr1` (baseline shipped) → later PRs add rows.

| Key | English source | Context / surface | Variables | Status |
|-----|----------------|-------------------|-----------|--------|
| `settings.appearance.language.label` | Language | Settings → Appearance → Preferences card, row label | — | in-pr1 |
| `settings.appearance.language.description` | Choose the interface language. | Same row, sub-copy | — | in-pr1 |
| `settings.appearance.language.option.system` | System | Language segmented control, “follow system” option | — | in-pr1 |
| `settings.appearance.language.option.en` | English | Language segmented control, English option | — | in-pr1 |
| `settings.appearance.language.option.zh-Hans` | 简体中文 | Language segmented control, Simplified Chinese option (endonym kept in both catalogs) | — | in-pr1 |
| `settings.appearance.font.size.label` | Font size | Font size row, label + control legend | — | in-pr1 |
| `settings.appearance.font.size.description` | Applies across conversations and interface text | Font size row, sub-copy | — | in-pr1 |
| `settings.appearance.font.size.option.smaller` | Smaller | Font size segmented control | — | in-pr1 |
| `settings.appearance.font.size.option.default` | Default | Font size segmented control | — | in-pr1 |
| `settings.appearance.font.size.option.larger` | Larger | Font size segmented control | — | in-pr1 |
| `onboarding.footer.back` | Back | Onboarding footer back button (OnboardingFooter / MachineOnboardingFlow chrome) | — | in-pr2 |
| `onboarding.footer.return-to-onboarding` | Return to onboarding | Onboarding footer back button (OnboardingFooter / MachineOnboardingFlow chrome) | — | in-pr2 |
| `onboarding.landing.tagline-line1` | Your people, your agents, your projects — | First-launch landing page (MachineOnboardingFlow identity page) | — | in-pr2 |
| `onboarding.landing.tagline-line2` | all in one place. | First-launch landing page (MachineOnboardingFlow identity page) | — | in-pr2 |
| `onboarding.landing.loading` | Loading identity… | First-launch landing page (MachineOnboardingFlow identity page) | — | in-pr2 |
| `onboarding.landing.continue-setup` | Continue setup | First-launch landing page (MachineOnboardingFlow identity page) | — | in-pr2 |
| `onboarding.landing.create-key` | Create a new identity key | First-launch landing page (MachineOnboardingFlow identity page) | — | in-pr2 |
| `onboarding.landing.use-different-key` | Use a different key instead | First-launch landing page (MachineOnboardingFlow identity page) | — | in-pr2 |
| `onboarding.landing.use-existing-key` | Use an existing key | First-launch landing page (MachineOnboardingFlow identity page) | — | in-pr2 |
| `onboarding.profile.title` | What should we call you? | Display-name step (ProfileStep) | — | in-pr2 |
| `onboarding.profile.body` | Pick the name people and agents will see in Buzz. You can change it anytime. | Display-name step (ProfileStep) | — | in-pr2 |
| `onboarding.profile.name-label` | Name | Display-name step (ProfileStep) | — | in-pr2 |
| `onboarding.profile.name-placeholder` | Enter your name | Display-name step (ProfileStep) | — | in-pr2 |
| `onboarding.profile.aria-saving` | Saving profile | Display-name step (ProfileStep) | — | in-pr2 |
| `onboarding.profile.continue` | Continue | Display-name step (ProfileStep) | — | in-pr2 |
| `onboarding.profile.create-key` | Create an identity key | Display-name step (ProfileStep) | — | in-pr2 |
| `onboarding.profile.back` | Back | Display-name step (ProfileStep) | — | in-pr2 |
| `onboarding.profile.have-key` | I already have a key | Display-name step (ProfileStep) | — | in-pr2 |
| `onboarding.profile.skip` | Skip for now | Display-name step (ProfileStep) | — | in-pr2 |
| `onboarding.profile.continue-without-saving` | Continue without saving | Display-name step (ProfileStep) | — | in-pr2 |
| `onboarding.profile.toast-reconnect-failed` | Could not reconnect to the relay. {{detail}} | Display-name step (ProfileStep) | detail | in-pr2 |
| `onboarding.avatar.default-name` | Your avatar | Avatar step (AvatarStep) | — | in-pr2 |
| `onboarding.avatar.preview-aria` | {{name}} avatar | Avatar step (AvatarStep) | name | in-pr2 |
| `onboarding.avatar.add-image` | Add a display image | Avatar step (AvatarStep) | — | in-pr2 |
| `onboarding.avatar.skip` | Skip for now | Avatar step (AvatarStep) | — | in-pr2 |
| `onboarding.avatar.continue-without-saving` | Continue without saving | Avatar step (AvatarStep) | — | in-pr2 |
| `onboarding.avatar.back` | Back | Avatar step (AvatarStep) | — | in-pr2 |
| `onboarding.avatar.aria-saving-profile` | Saving profile | Avatar step (AvatarStep) | — | in-pr2 |
| `onboarding.avatar.aria-uploading` | Uploading avatar | Avatar step (AvatarStep) | — | in-pr2 |
| `onboarding.avatar.next` | Next | Avatar step (AvatarStep) | — | in-pr2 |
| `onboarding.avatar.title` | Next, add a display image | Avatar step (AvatarStep) | — | in-pr2 |
| `onboarding.avatar.body` | Choose an image or emoji as your avatar | Avatar step (AvatarStep) | — | in-pr2 |
| `onboarding.key-import.back` | Back | Key import + recovery entry points (NostrKeyImportForm / NsecMaskedDisplay / OnboardingFlow / MachineOnboardingFlow) | — | in-pr2 |
| `onboarding.key-import.confirm-new-identity` | This will create a new identity and abandon your previous key. This cannot be undone. Continue? | Key import + recovery entry points (NostrKeyImportForm / NsecMaskedDisplay / OnboardingFlow / MachineOnboardingFlow) | — | in-pr2 |
| `onboarding.key-import.error-new-identity` | Failed to create a new identity. Please try again. | Key import + recovery entry points (NostrKeyImportForm / NsecMaskedDisplay / OnboardingFlow / MachineOnboardingFlow) | — | in-pr2 |
| `onboarding.key-import.label-private-key` | Private key | Key import + recovery entry points (NostrKeyImportForm / NsecMaskedDisplay / OnboardingFlow / MachineOnboardingFlow) | — | in-pr2 |
| `onboarding.key-import.placeholder-key` | Enter your key here | Key import + recovery entry points (NostrKeyImportForm / NsecMaskedDisplay / OnboardingFlow / MachineOnboardingFlow) | — | in-pr2 |
| `onboarding.key-import.aria-hide-key` | Hide private key | Key import + recovery entry points (NostrKeyImportForm / NsecMaskedDisplay / OnboardingFlow / MachineOnboardingFlow) | — | in-pr2 |
| `onboarding.key-import.aria-reveal-key` | Reveal private key | Key import + recovery entry points (NostrKeyImportForm / NsecMaskedDisplay / OnboardingFlow / MachineOnboardingFlow) | — | in-pr2 |
| `onboarding.key-import.nsec-actions-aria` | Private key actions | Key import + recovery entry points (NostrKeyImportForm / NsecMaskedDisplay / OnboardingFlow / MachineOnboardingFlow) | — | in-pr2 |
| `onboarding.key-import.nsec-copy` | Copy | Key import + recovery entry points (NostrKeyImportForm / NsecMaskedDisplay / OnboardingFlow / MachineOnboardingFlow) | — | in-pr2 |
| `onboarding.key-import.nsec-copy-aria` | Copy private key | Key import + recovery entry points (NostrKeyImportForm / NsecMaskedDisplay / OnboardingFlow / MachineOnboardingFlow) | — | in-pr2 |
| `onboarding.key-import.choose-backup-file` | Choose a backup file | Key import + recovery entry points (NostrKeyImportForm / NsecMaskedDisplay / OnboardingFlow / MachineOnboardingFlow) | — | in-pr2 |
| `onboarding.key-import.drop-backup-here` | Drop your backup file here | Key import + recovery entry points (NostrKeyImportForm / NsecMaskedDisplay / OnboardingFlow / MachineOnboardingFlow) | — | in-pr2 |
| `onboarding.key-import.drop-key-here` | Drop a key here | Key import + recovery entry points (NostrKeyImportForm / NsecMaskedDisplay / OnboardingFlow / MachineOnboardingFlow) | — | in-pr2 |
| `onboarding.key-import.backup-password` | Backup password | Key import + recovery entry points (NostrKeyImportForm / NsecMaskedDisplay / OnboardingFlow / MachineOnboardingFlow) | — | in-pr2 |
| `onboarding.key-import.aria-hide-password` | Hide password | Key import + recovery entry points (NostrKeyImportForm / NsecMaskedDisplay / OnboardingFlow / MachineOnboardingFlow) | — | in-pr2 |
| `onboarding.key-import.aria-reveal-password` | Reveal password | Key import + recovery entry points (NostrKeyImportForm / NsecMaskedDisplay / OnboardingFlow / MachineOnboardingFlow) | — | in-pr2 |
| `onboarding.key-import.npub-found` | Nostr identity found | Key import + recovery entry points (NostrKeyImportForm / NsecMaskedDisplay / OnboardingFlow / MachineOnboardingFlow) | — | in-pr2 |
| `onboarding.key-import.npub-preview` | This will use this Nostr identity: | Key import + recovery entry points (NostrKeyImportForm / NsecMaskedDisplay / OnboardingFlow / MachineOnboardingFlow) | — | in-pr2 |
| `onboarding.key-import.waiting-ncryptsec` | Waiting for a complete ncryptsec backup | Key import + recovery entry points (NostrKeyImportForm / NsecMaskedDisplay / OnboardingFlow / MachineOnboardingFlow) | — | in-pr2 |
| `onboarding.key-import.waiting-nsec` | Waiting for a valid nsec1 key | Key import + recovery entry points (NostrKeyImportForm / NsecMaskedDisplay / OnboardingFlow / MachineOnboardingFlow) | — | in-pr2 |
| `onboarding.key-import.error-file-too-large` | That file is too large to be a key backup or private key. Choose another file. | Key import + recovery entry points (NostrKeyImportForm / NsecMaskedDisplay / OnboardingFlow / MachineOnboardingFlow) | — | in-pr2 |
| `onboarding.key-import.error-read-file` | Couldn't read that file. | Key import + recovery entry points (NostrKeyImportForm / NsecMaskedDisplay / OnboardingFlow / MachineOnboardingFlow) | — | in-pr2 |
| `onboarding.key-import.error-enter-password` | Enter the password for this key backup. | Key import + recovery entry points (NostrKeyImportForm / NsecMaskedDisplay / OnboardingFlow / MachineOnboardingFlow) | — | in-pr2 |
| `onboarding.key-import.error-incomplete-ncryptsec` | That doesn't look like a complete ncryptsec backup. | Key import + recovery entry points (NostrKeyImportForm / NsecMaskedDisplay / OnboardingFlow / MachineOnboardingFlow) | — | in-pr2 |
| `onboarding.key-import.error-invalid-nsec` | That doesn't look like a valid nsec. Paste an nsec1 key. | Key import + recovery entry points (NostrKeyImportForm / NsecMaskedDisplay / OnboardingFlow / MachineOnboardingFlow) | — | in-pr2 |
| `onboarding.key-import.error-import` | Couldn't import this key. | Key import + recovery entry points (NostrKeyImportForm / NsecMaskedDisplay / OnboardingFlow / MachineOnboardingFlow) | — | in-pr2 |
| `onboarding.key-import.aria-importing` | Importing key | Key import + recovery entry points (NostrKeyImportForm / NsecMaskedDisplay / OnboardingFlow / MachineOnboardingFlow) | — | in-pr2 |
| `onboarding.key-import.next` | Next | Key import + recovery entry points (NostrKeyImportForm / NsecMaskedDisplay / OnboardingFlow / MachineOnboardingFlow) | — | in-pr2 |
| `onboarding.key-import.continue-with-key` | Continue with this key | Key import + recovery entry points (NostrKeyImportForm / NsecMaskedDisplay / OnboardingFlow / MachineOnboardingFlow) | — | in-pr2 |
| `onboarding.key-import.start-new-identity` | Start new identity | Key import + recovery entry points (NostrKeyImportForm / NsecMaskedDisplay / OnboardingFlow / MachineOnboardingFlow) | — | in-pr2 |
| `onboarding.key-import.title-reimport` | Re-import your key | Key import + recovery entry points (NostrKeyImportForm / NsecMaskedDisplay / OnboardingFlow / MachineOnboardingFlow) | — | in-pr2 |
| `onboarding.key-import.body-reimport` | Your identity is no longer in the system keyring. Re-import your nsec to restore it — Buzz will restart to finish recovery. Or go back to start a new identity with a fresh key. | Key import + recovery entry points (NostrKeyImportForm / NsecMaskedDisplay / OnboardingFlow / MachineOnboardingFlow) | — | in-pr2 |
| `onboarding.key-import.title-existing` | Use your existing key | Key import + recovery entry points (NostrKeyImportForm / NsecMaskedDisplay / OnboardingFlow / MachineOnboardingFlow) | — | in-pr2 |
| `onboarding.key-import.body-existing` | Import your Nostr private key to use that identity with Buzz. If this key already has a profile on the relay, your name and avatar are restored automatically. | Key import + recovery entry points (NostrKeyImportForm / NsecMaskedDisplay / OnboardingFlow / MachineOnboardingFlow) | — | in-pr2 |
| `onboarding.key-import.title-restore-backup` | Restore from a backup file | Key import + recovery entry points (NostrKeyImportForm / NsecMaskedDisplay / OnboardingFlow / MachineOnboardingFlow) | — | in-pr2 |
| `onboarding.key-import.body-restore-backup` | Choose the encrypted backup file you saved from Buzz. | Key import + recovery entry points (NostrKeyImportForm / NsecMaskedDisplay / OnboardingFlow / MachineOnboardingFlow) | — | in-pr2 |
| `onboarding.key-import.title-recover-phone` | Recover from your phone | Key import + recovery entry points (NostrKeyImportForm / NsecMaskedDisplay / OnboardingFlow / MachineOnboardingFlow) | — | in-pr2 |
| `onboarding.key-import.title-scan-signin` | Scan to sign in | Key import + recovery entry points (NostrKeyImportForm / NsecMaskedDisplay / OnboardingFlow / MachineOnboardingFlow) | — | in-pr2 |
| `onboarding.key-import.body-scan-qr` | Scan this code with a device where you’re currently signed in to Buzz. | Key import + recovery entry points (NostrKeyImportForm / NsecMaskedDisplay / OnboardingFlow / MachineOnboardingFlow) | — | in-pr2 |
| `onboarding.key-import.body-confirm-code` | Confirm the code before sharing your identity. | Key import + recovery entry points (NostrKeyImportForm / NsecMaskedDisplay / OnboardingFlow / MachineOnboardingFlow) | — | in-pr2 |
| `onboarding.key-import.title-unlock` | Unlock your account | Key import + recovery entry points (NostrKeyImportForm / NsecMaskedDisplay / OnboardingFlow / MachineOnboardingFlow) | — | in-pr2 |
| `onboarding.key-import.title-enter-key` | Enter your private key | Key import + recovery entry points (NostrKeyImportForm / NsecMaskedDisplay / OnboardingFlow / MachineOnboardingFlow) | — | in-pr2 |
| `onboarding.key-import.body-backup-password` | Enter your backup password to restore your identity. | Key import + recovery entry points (NostrKeyImportForm / NsecMaskedDisplay / OnboardingFlow / MachineOnboardingFlow) | — | in-pr2 |
| `onboarding.key-import.enter-key-intro` | Paste your private key to sign in to Buzz. You can also use a  | Key import + recovery entry points (NostrKeyImportForm / NsecMaskedDisplay / OnboardingFlow / MachineOnboardingFlow) — sentence fragment, keeps exact DOM (no <Trans>) | — | in-pr2 |
| `onboarding.key-import.link-backup-file` | backup file | Key import + recovery entry points (NostrKeyImportForm / NsecMaskedDisplay / OnboardingFlow / MachineOnboardingFlow) — sentence fragment, keeps exact DOM (no <Trans>) | — | in-pr2 |
| `onboarding.key-import.or` | , or  | Key import + recovery entry points (NostrKeyImportForm / NsecMaskedDisplay / OnboardingFlow / MachineOnboardingFlow) — sentence fragment, keeps exact DOM (no <Trans>) | — | in-pr2 |
| `onboarding.key-import.link-recover-phone` | recover from your phone | Key import + recovery entry points (NostrKeyImportForm / NsecMaskedDisplay / OnboardingFlow / MachineOnboardingFlow) — sentence fragment, keeps exact DOM (no <Trans>) | — | in-pr2 |
| `onboarding.key-import.terminal` | . | Key import + recovery entry points (NostrKeyImportForm / NsecMaskedDisplay / OnboardingFlow / MachineOnboardingFlow) — sentence fragment, keeps exact DOM (no <Trans>) | — | in-pr2 |
| `onboarding.recovery.relaunch` | Relaunch Buzz | Recovery screens (RecoveryScreen / RelaunchRequiredScreen / ResetFailedScreen / KeyringLockedScreen / IdentityRecoveryPairing) | — | in-pr2 |
| `onboarding.recovery.keyring-title` | Unlock your system keyring | Recovery screens (RecoveryScreen / RelaunchRequiredScreen / ResetFailedScreen / KeyringLockedScreen / IdentityRecoveryPairing) | — | in-pr2 |
| `onboarding.recovery.keyring-body` | Your identity is safe in the OS keyring, but it's unreachable this session. Unlock your keyring or sign into your desktop session, then relaunch Buzz. | Recovery screens (RecoveryScreen / RelaunchRequiredScreen / ResetFailedScreen / KeyringLockedScreen / IdentityRecoveryPairing) | — | in-pr2 |
| `onboarding.recovery.keyring-reimport` | Re-import your key instead | Recovery screens (RecoveryScreen / RelaunchRequiredScreen / ResetFailedScreen / KeyringLockedScreen / IdentityRecoveryPairing) | — | in-pr2 |
| `onboarding.recovery.keyring-cancel` | Cancel | Recovery screens (RecoveryScreen / RelaunchRequiredScreen / ResetFailedScreen / KeyringLockedScreen / IdentityRecoveryPairing) | — | in-pr2 |
| `onboarding.recovery.keyring-confirm` | Importing a different nsec replaces the identity currently locked in the keyring for this install. The previous identity will no longer be accessible. Continue? | Recovery screens (RecoveryScreen / RelaunchRequiredScreen / ResetFailedScreen / KeyringLockedScreen / IdentityRecoveryPairing) | — | in-pr2 |
| `onboarding.recovery.title-restart` | Restart Buzz to finish recovery | Recovery screens (RecoveryScreen / RelaunchRequiredScreen / ResetFailedScreen / KeyringLockedScreen / IdentityRecoveryPairing) | — | in-pr2 |
| `onboarding.recovery.body-restart` | Your identity was updated. Buzz needs to restart so syncing and agents run under it. | Recovery screens (RecoveryScreen / RelaunchRequiredScreen / ResetFailedScreen / KeyringLockedScreen / IdentityRecoveryPairing) | — | in-pr2 |
| `onboarding.recovery.title-signout-failed` | Sign out could not complete | Recovery screens (RecoveryScreen / RelaunchRequiredScreen / ResetFailedScreen / KeyringLockedScreen / IdentityRecoveryPairing) | — | in-pr2 |
| `onboarding.recovery.body-signout-failed` | Buzz was unable to fully clear your local data. Try relaunching — the reset will resume automatically. If this persists, contact support. | Recovery screens (RecoveryScreen / RelaunchRequiredScreen / ResetFailedScreen / KeyringLockedScreen / IdentityRecoveryPairing) | — | in-pr2 |
| `onboarding.recovery.qr-title` | Desktop identity recovery QR code | Recovery screens (RecoveryScreen / RelaunchRequiredScreen / ResetFailedScreen / KeyringLockedScreen / IdentityRecoveryPairing) | — | in-pr2 |
| `onboarding.recovery.sas-question` | Does this code match your phone? | Recovery screens (RecoveryScreen / RelaunchRequiredScreen / ResetFailedScreen / KeyringLockedScreen / IdentityRecoveryPairing) | — | in-pr2 |
| `onboarding.recovery.sas-warning` | This gives this desktop permanent access to your Buzz identity. Only continue if you trust it. | Recovery screens (RecoveryScreen / RelaunchRequiredScreen / ResetFailedScreen / KeyringLockedScreen / IdentityRecoveryPairing) | — | in-pr2 |
| `onboarding.recovery.sas-match` | Codes match | Recovery screens (RecoveryScreen / RelaunchRequiredScreen / ResetFailedScreen / KeyringLockedScreen / IdentityRecoveryPairing) | — | in-pr2 |
| `onboarding.recovery.cancel` | Cancel | Recovery screens (RecoveryScreen / RelaunchRequiredScreen / ResetFailedScreen / KeyringLockedScreen / IdentityRecoveryPairing) | — | in-pr2 |
| `onboarding.recovery.done` | Identity received securely | Recovery screens (RecoveryScreen / RelaunchRequiredScreen / ResetFailedScreen / KeyringLockedScreen / IdentityRecoveryPairing) | — | in-pr2 |
| `onboarding.recovery.retry` | Try again | Recovery screens (RecoveryScreen / RelaunchRequiredScreen / ResetFailedScreen / KeyringLockedScreen / IdentityRecoveryPairing) | — | in-pr2 |
| `onboarding.recovery.receiving` | Receiving identity from mobile device... | Recovery screens (RecoveryScreen / RelaunchRequiredScreen / ResetFailedScreen / KeyringLockedScreen / IdentityRecoveryPairing) | — | in-pr2 |
| `onboarding.recovery.starting` | Starting pairing... | Recovery screens (RecoveryScreen / RelaunchRequiredScreen / ResetFailedScreen / KeyringLockedScreen / IdentityRecoveryPairing) | — | in-pr2 |
| `onboarding.recovery.generating` | Generating pairing code... | Recovery screens (RecoveryScreen / RelaunchRequiredScreen / ResetFailedScreen / KeyringLockedScreen / IdentityRecoveryPairing) | — | in-pr2 |
| `onboarding.recovery.copied` | Copied | Recovery screens (RecoveryScreen / RelaunchRequiredScreen / ResetFailedScreen / KeyringLockedScreen / IdentityRecoveryPairing) | — | in-pr2 |
| `onboarding.recovery.copy-code` | Copy pairing code | Recovery screens (RecoveryScreen / RelaunchRequiredScreen / ResetFailedScreen / KeyringLockedScreen / IdentityRecoveryPairing) | — | in-pr2 |
| `onboarding.recovery.error-code-expired` | This pairing code expired or lost its connection. Create a new code and try again. | Recovery screens (RecoveryScreen / RelaunchRequiredScreen / ResetFailedScreen / KeyringLockedScreen / IdentityRecoveryPairing) | — | in-pr2 |
| `onboarding.recovery.error-start` | Could not start recovery. | Recovery screens (RecoveryScreen / RelaunchRequiredScreen / ResetFailedScreen / KeyringLockedScreen / IdentityRecoveryPairing) | — | in-pr2 |
| `onboarding.recovery.error-stopped` | Recovery stopped: {{reason}} | Recovery screens (RecoveryScreen / RelaunchRequiredScreen / ResetFailedScreen / KeyringLockedScreen / IdentityRecoveryPairing) | reason | in-pr2 |
| `onboarding.recovery.error-copy` | Could not copy the pairing code. Try again. | Recovery screens (RecoveryScreen / RelaunchRequiredScreen / ResetFailedScreen / KeyringLockedScreen / IdentityRecoveryPairing) | — | in-pr2 |
| `onboarding.recovery.error-mismatch` | The codes didn't match. Pairing was canceled. | Recovery screens (RecoveryScreen / RelaunchRequiredScreen / ResetFailedScreen / KeyringLockedScreen / IdentityRecoveryPairing) | — | in-pr2 |
| `onboarding.recovery.error-confirm` | Could not confirm recovery. | Recovery screens (RecoveryScreen / RelaunchRequiredScreen / ResetFailedScreen / KeyringLockedScreen / IdentityRecoveryPairing) | — | in-pr2 |
| `onboarding.recovery.error-load-identity` | Failed to load identity | Recovery screens (RecoveryScreen / RelaunchRequiredScreen / ResetFailedScreen / KeyringLockedScreen / IdentityRecoveryPairing) | — | in-pr2 |
| `onboarding.recovery.error-save-identity` | Failed to save identity | Recovery screens (RecoveryScreen / RelaunchRequiredScreen / ResetFailedScreen / KeyringLockedScreen / IdentityRecoveryPairing) | — | in-pr2 |
| `onboarding.backup.title-create` | Create a secure backup file | Backup step + encrypted backup (BackupStep / EncryptedBackupCreator / BackupTestFlow / DownloadKeyStep) | — | in-pr2 |
| `onboarding.backup.title-ready` | Your backup is ready | Backup step + encrypted backup (BackupStep / EncryptedBackupCreator / BackupTestFlow / DownloadKeyStep) | — | in-pr2 |
| `onboarding.backup.title-verify` | Verify your backup | Backup step + encrypted backup (BackupStep / EncryptedBackupCreator / BackupTestFlow / DownloadKeyStep) | — | in-pr2 |
| `onboarding.backup.title-verified` | Your backup is verified | Backup step + encrypted backup (BackupStep / EncryptedBackupCreator / BackupTestFlow / DownloadKeyStep) | — | in-pr2 |
| `onboarding.backup.body-create` | This creates a password-protected file with your private key. Remember, Buzz can’t recover your key if you lose it. | Backup step + encrypted backup (BackupStep / EncryptedBackupCreator / BackupTestFlow / DownloadKeyStep) | — | in-pr2 |
| `onboarding.backup.body-ready` | Test your backup to make sure it works, or continue without testing. | Backup step + encrypted backup (BackupStep / EncryptedBackupCreator / BackupTestFlow / DownloadKeyStep) | — | in-pr2 |
| `onboarding.backup.body-verify` | Enter your password to make sure you can unlock this file. | Backup step + encrypted backup (BackupStep / EncryptedBackupCreator / BackupTestFlow / DownloadKeyStep) | — | in-pr2 |
| `onboarding.backup.body-verified` | Your file and password can restore your identity. | Backup step + encrypted backup (BackupStep / EncryptedBackupCreator / BackupTestFlow / DownloadKeyStep) | — | in-pr2 |
| `onboarding.backup.continue` | Continue | Backup step + encrypted backup (BackupStep / EncryptedBackupCreator / BackupTestFlow / DownloadKeyStep) | — | in-pr2 |
| `onboarding.backup.skip` | Skip for now | Backup step + encrypted backup (BackupStep / EncryptedBackupCreator / BackupTestFlow / DownloadKeyStep) | — | in-pr2 |
| `onboarding.backup.error-retrieve-key` | Failed to retrieve private key. | Backup step + encrypted backup (BackupStep / EncryptedBackupCreator / BackupTestFlow / DownloadKeyStep) | — | in-pr2 |
| `onboarding.backup.storage-keychain-title` | Protected by your system keychain | Backup step + encrypted backup (BackupStep / EncryptedBackupCreator / BackupTestFlow / DownloadKeyStep) | — | in-pr2 |
| `onboarding.backup.storage-keychain-desc` | Buzz keeps your identity key in your system keychain. Your computer may ask for your password when Buzz needs to read the key. | Backup step + encrypted backup (BackupStep / EncryptedBackupCreator / BackupTestFlow / DownloadKeyStep) | — | in-pr2 |
| `onboarding.backup.storage-localfile-title` | Stored in private device storage | Backup step + encrypted backup (BackupStep / EncryptedBackupCreator / BackupTestFlow / DownloadKeyStep) | — | in-pr2 |
| `onboarding.backup.storage-localfile-desc` | Your system keychain wasn't available, so Buzz keeps your identity key in a private file on this device. | Backup step + encrypted backup (BackupStep / EncryptedBackupCreator / BackupTestFlow / DownloadKeyStep) | — | in-pr2 |
| `onboarding.backup.storage-default-title` | Protected in private device storage | Backup step + encrypted backup (BackupStep / EncryptedBackupCreator / BackupTestFlow / DownloadKeyStep) | — | in-pr2 |
| `onboarding.backup.storage-default-desc` | Buzz keeps your identity key protected on this device. Make a separate backup in case you lose access. | Backup step + encrypted backup (BackupStep / EncryptedBackupCreator / BackupTestFlow / DownloadKeyStep) | — | in-pr2 |
| `onboarding.backup.options-title` | Backup options | Backup step + encrypted backup (BackupStep / EncryptedBackupCreator / BackupTestFlow / DownloadKeyStep) | — | in-pr2 |
| `onboarding.backup.options-body` | Your identity key works like a password for your Buzz account. Keep a copy somewhere safe. You can create a backup file and lock it with a password you can remember. | Backup step + encrypted backup (BackupStep / EncryptedBackupCreator / BackupTestFlow / DownloadKeyStep) | — | in-pr2 |
| `onboarding.backup.password-manager-title` | Saved in your password manager | Backup step + encrypted backup (BackupStep / EncryptedBackupCreator / BackupTestFlow / DownloadKeyStep) | — | in-pr2 |
| `onboarding.backup.password-manager-desc` | Copy your identity key, then save it in a password manager like 1Password. | Backup step + encrypted backup (BackupStep / EncryptedBackupCreator / BackupTestFlow / DownloadKeyStep) | — | in-pr2 |
| `onboarding.backup.copying` | Copying… | Backup step + encrypted backup (BackupStep / EncryptedBackupCreator / BackupTestFlow / DownloadKeyStep) | — | in-pr2 |
| `onboarding.backup.copied` | Copied to clipboard | Backup step + encrypted backup (BackupStep / EncryptedBackupCreator / BackupTestFlow / DownloadKeyStep) | — | in-pr2 |
| `onboarding.backup.copy` | Copy to clipboard | Backup step + encrypted backup (BackupStep / EncryptedBackupCreator / BackupTestFlow / DownloadKeyStep) | — | in-pr2 |
| `onboarding.backup.locked-file-title` | Locked in a backup file | Backup step + encrypted backup (BackupStep / EncryptedBackupCreator / BackupTestFlow / DownloadKeyStep) | — | in-pr2 |
| `onboarding.backup.locked-file-desc` | Create a backup file and choose a password you can remember. You'll need both to restore your account. | Backup step + encrypted backup (BackupStep / EncryptedBackupCreator / BackupTestFlow / DownloadKeyStep) | — | in-pr2 |
| `onboarding.backup.create-locked-backup` | Create locked backup | Backup step + encrypted backup (BackupStep / EncryptedBackupCreator / BackupTestFlow / DownloadKeyStep) | — | in-pr2 |
| `onboarding.backup.copy-error` | Could not retrieve your private key: {{error}}. You can continue and find it later in Settings > Profile > Identity. | Backup step + encrypted backup (BackupStep / EncryptedBackupCreator / BackupTestFlow / DownloadKeyStep) | error | in-pr2 |
| `onboarding.backup.title-creating` | Creating your identity key | Backup step + encrypted backup (BackupStep / EncryptedBackupCreator / BackupTestFlow / DownloadKeyStep) | — | in-pr2 |
| `onboarding.backup.title-created` | Your private identity key | Backup step + encrypted backup (BackupStep / EncryptedBackupCreator / BackupTestFlow / DownloadKeyStep) | — | in-pr2 |
| `onboarding.backup.share-warning` | Don't share this key. Anyone who has it can access your account. | Backup step + encrypted backup (BackupStep / EncryptedBackupCreator / BackupTestFlow / DownloadKeyStep) | — | in-pr2 |
| `onboarding.backup.separator-spaces` | Spaces | Backup step + encrypted backup (BackupStep / EncryptedBackupCreator / BackupTestFlow / DownloadKeyStep) | — | in-pr2 |
| `onboarding.backup.separator-hyphens` | Hyphens | Backup step + encrypted backup (BackupStep / EncryptedBackupCreator / BackupTestFlow / DownloadKeyStep) | — | in-pr2 |
| `onboarding.backup.separator-periods` | Periods | Backup step + encrypted backup (BackupStep / EncryptedBackupCreator / BackupTestFlow / DownloadKeyStep) | — | in-pr2 |
| `onboarding.backup.separator-commas` | Commas | Backup step + encrypted backup (BackupStep / EncryptedBackupCreator / BackupTestFlow / DownloadKeyStep) | — | in-pr2 |
| `onboarding.backup.error-generate` | Failed to generate a password. | Backup step + encrypted backup (BackupStep / EncryptedBackupCreator / BackupTestFlow / DownloadKeyStep) | — | in-pr2 |
| `onboarding.backup.generate-password` | Generate a password | Backup step + encrypted backup (BackupStep / EncryptedBackupCreator / BackupTestFlow / DownloadKeyStep) | — | in-pr2 |
| `onboarding.backup.words` | Words | Backup step + encrypted backup (BackupStep / EncryptedBackupCreator / BackupTestFlow / DownloadKeyStep) | — | in-pr2 |
| `onboarding.backup.separator` | Separator | Backup step + encrypted backup (BackupStep / EncryptedBackupCreator / BackupTestFlow / DownloadKeyStep) | — | in-pr2 |
| `onboarding.backup.password-label` | Password | Backup step + encrypted backup (BackupStep / EncryptedBackupCreator / BackupTestFlow / DownloadKeyStep) | — | in-pr2 |
| `onboarding.backup.aria-encryption-password` | Encryption password | Backup step + encrypted backup (BackupStep / EncryptedBackupCreator / BackupTestFlow / DownloadKeyStep) | — | in-pr2 |
| `onboarding.backup.placeholder-password-min` | Password (min {{minLength}} characters) | Backup step + encrypted backup (BackupStep / EncryptedBackupCreator / BackupTestFlow / DownloadKeyStep) | minLength | in-pr2 |
| `onboarding.backup.saved-hidden` | Backup password saved; hidden for security. | Backup step + encrypted backup (BackupStep / EncryptedBackupCreator / BackupTestFlow / DownloadKeyStep) | — | in-pr2 |
| `onboarding.backup.aria-change-saved-password` | Change saved backup password | Backup step + encrypted backup (BackupStep / EncryptedBackupCreator / BackupTestFlow / DownloadKeyStep) | — | in-pr2 |
| `onboarding.backup.aria-hide-password` | Hide password | Backup step + encrypted backup (BackupStep / EncryptedBackupCreator / BackupTestFlow / DownloadKeyStep) | — | in-pr2 |
| `onboarding.backup.aria-reveal-password` | Reveal password | Backup step + encrypted backup (BackupStep / EncryptedBackupCreator / BackupTestFlow / DownloadKeyStep) | — | in-pr2 |
| `onboarding.backup.saved-to` | Backup saved to {{path}} | Backup step + encrypted backup (BackupStep / EncryptedBackupCreator / BackupTestFlow / DownloadKeyStep) | path | in-pr2 |
| `onboarding.backup.saved-note` | Your password isn't kept — download another copy anytime, or start over to choose a new password. | Backup step + encrypted backup (BackupStep / EncryptedBackupCreator / BackupTestFlow / DownloadKeyStep) | — | in-pr2 |
| `onboarding.backup.aria-encrypting` | Encrypting your key | Backup step + encrypted backup (BackupStep / EncryptedBackupCreator / BackupTestFlow / DownloadKeyStep) | — | in-pr2 |
| `onboarding.backup.download-again` | Download backup again | Backup step + encrypted backup (BackupStep / EncryptedBackupCreator / BackupTestFlow / DownloadKeyStep) | — | in-pr2 |
| `onboarding.backup.save-backup` | Save backup | Backup step + encrypted backup (BackupStep / EncryptedBackupCreator / BackupTestFlow / DownloadKeyStep) | — | in-pr2 |
| `onboarding.backup.change-password-title` | Create a new backup password? | Backup step + encrypted backup (BackupStep / EncryptedBackupCreator / BackupTestFlow / DownloadKeyStep) | — | in-pr2 |
| `onboarding.backup.change-password-body` | Starting over lets you pick a new password and download a fresh backup file. Backups you saved earlier will still work — just use the password you created them with. | Backup step + encrypted backup (BackupStep / EncryptedBackupCreator / BackupTestFlow / DownloadKeyStep) | — | in-pr2 |
| `onboarding.backup.keep-current` | Keep current backup | Backup step + encrypted backup (BackupStep / EncryptedBackupCreator / BackupTestFlow / DownloadKeyStep) | — | in-pr2 |
| `onboarding.backup.start-new-password` | Start with a new password | Backup step + encrypted backup (BackupStep / EncryptedBackupCreator / BackupTestFlow / DownloadKeyStep) | — | in-pr2 |
| `onboarding.backup.error-encrypt` | Failed to encrypt your key. | Backup step + encrypted backup (BackupStep / EncryptedBackupCreator / BackupTestFlow / DownloadKeyStep) | — | in-pr2 |
| `onboarding.backup.error-save` | Failed to save your key. | Backup step + encrypted backup (BackupStep / EncryptedBackupCreator / BackupTestFlow / DownloadKeyStep) | — | in-pr2 |
| `onboarding.backup.test-title` | Test your backup | Backup step + encrypted backup (BackupStep / EncryptedBackupCreator / BackupTestFlow / DownloadKeyStep) | — | in-pr2 |
| `onboarding.backup.select-file` | Select your backup file | Backup step + encrypted backup (BackupStep / EncryptedBackupCreator / BackupTestFlow / DownloadKeyStep) | — | in-pr2 |
| `onboarding.backup.drop-here` | Drop your backup file here | Backup step + encrypted backup (BackupStep / EncryptedBackupCreator / BackupTestFlow / DownloadKeyStep) | — | in-pr2 |
| `onboarding.backup.re-download` | Re-download backup | Backup step + encrypted backup (BackupStep / EncryptedBackupCreator / BackupTestFlow / DownloadKeyStep) | — | in-pr2 |
| `onboarding.backup.test-error-read` | Could not read that file. | Backup step + encrypted backup (BackupStep / EncryptedBackupCreator / BackupTestFlow / DownloadKeyStep) | — | in-pr2 |
| `onboarding.backup.test-error-not-backup` | That doesn't look like a key backup file. | Backup step + encrypted backup (BackupStep / EncryptedBackupCreator / BackupTestFlow / DownloadKeyStep) | — | in-pr2 |
| `onboarding.backup.test-error-not-your-backup` | That doesn't look like your key backup. Choose the file you just downloaded. | Backup step + encrypted backup (BackupStep / EncryptedBackupCreator / BackupTestFlow / DownloadKeyStep) | — | in-pr2 |
| `onboarding.backup.test-error-wrong-file` | That's a key backup, but not the one you just downloaded. | Backup step + encrypted backup (BackupStep / EncryptedBackupCreator / BackupTestFlow / DownloadKeyStep) | — | in-pr2 |
| `onboarding.backup.test-error-verify` | Could not verify this backup. | Backup step + encrypted backup (BackupStep / EncryptedBackupCreator / BackupTestFlow / DownloadKeyStep) | — | in-pr2 |
| `onboarding.backup.test-error-retrieve-key` | Could not retrieve your key. | Backup step + encrypted backup (BackupStep / EncryptedBackupCreator / BackupTestFlow / DownloadKeyStep) | — | in-pr2 |
| `onboarding.backup.test-success-title` | Your backup works! | Backup step + encrypted backup (BackupStep / EncryptedBackupCreator / BackupTestFlow / DownloadKeyStep) | — | in-pr2 |
| `onboarding.backup.test-success-body` | File and password verified. Keep them both somewhere safe — that's all you need to restore your identity. | Backup step + encrypted backup (BackupStep / EncryptedBackupCreator / BackupTestFlow / DownloadKeyStep) | — | in-pr2 |
| `onboarding.backup.test-aria-hide-key` | Hide unlocked private key | Backup step + encrypted backup (BackupStep / EncryptedBackupCreator / BackupTestFlow / DownloadKeyStep) | — | in-pr2 |
| `onboarding.backup.test-aria-reveal-key` | Reveal unlocked private key | Backup step + encrypted backup (BackupStep / EncryptedBackupCreator / BackupTestFlow / DownloadKeyStep) | — | in-pr2 |
| `onboarding.backup.test-success-generic-title` | This backup works | Backup step + encrypted backup (BackupStep / EncryptedBackupCreator / BackupTestFlow / DownloadKeyStep) | — | in-pr2 |
| `onboarding.backup.test-success-current` | It restores your current Buzz identity. | Backup step + encrypted backup (BackupStep / EncryptedBackupCreator / BackupTestFlow / DownloadKeyStep) | — | in-pr2 |
| `onboarding.backup.test-success-different` | It restores a different identity than the one signed in here. | Backup step + encrypted backup (BackupStep / EncryptedBackupCreator / BackupTestFlow / DownloadKeyStep) | — | in-pr2 |
| `onboarding.backup.test-another` | Test another backup | Backup step + encrypted backup (BackupStep / EncryptedBackupCreator / BackupTestFlow / DownloadKeyStep) | — | in-pr2 |
| `onboarding.backup.test-placeholder` | Your backup password | Backup step + encrypted backup (BackupStep / EncryptedBackupCreator / BackupTestFlow / DownloadKeyStep) | — | in-pr2 |
| `onboarding.backup.test-enter-password` | Enter the password to prove you can unlock this backup. | Backup step + encrypted backup (BackupStep / EncryptedBackupCreator / BackupTestFlow / DownloadKeyStep) | — | in-pr2 |
| `onboarding.backup.checking` | Checking… | Backup step + encrypted backup (BackupStep / EncryptedBackupCreator / BackupTestFlow / DownloadKeyStep) | — | in-pr2 |
| `onboarding.backup.verify` | Verify backup | Backup step + encrypted backup (BackupStep / EncryptedBackupCreator / BackupTestFlow / DownloadKeyStep) | — | in-pr2 |
| `onboarding.key-intro.title` | Create a private identity key | Identity-key intro + help dialog (IdentityKeyIntroduction / IdentityKeyHelpDialog) | — | in-pr2 |
| `onboarding.key-intro.body` | This key will be how you log into Buzz. You can use it across Buzz communities and other platforms. | Identity-key intro + help dialog (IdentityKeyIntroduction / IdentityKeyHelpDialog) | — | in-pr2 |
| `onboarding.key-intro.guidance-stored` | Stored securely on this device | Identity-key intro + help dialog (IdentityKeyIntroduction / IdentityKeyHelpDialog) | — | in-pr2 |
| `onboarding.key-intro.guidance-never-share` | Never share it—anyone with this key can sign in as you | Identity-key intro + help dialog (IdentityKeyIntroduction / IdentityKeyHelpDialog) | — | in-pr2 |
| `onboarding.key-intro.guidance-backup` | Use a secure backup to recover your account | Identity-key intro + help dialog (IdentityKeyIntroduction / IdentityKeyHelpDialog) | — | in-pr2 |
| `onboarding.key-intro.create-button` | Create my private key | Identity-key intro + help dialog (IdentityKeyIntroduction / IdentityKeyHelpDialog) | — | in-pr2 |
| `onboarding.key-intro.creating` | Creating key… | Identity-key intro + help dialog (IdentityKeyIntroduction / IdentityKeyHelpDialog) | — | in-pr2 |
| `onboarding.key-intro.help-trigger-inline` | Learn how identity keys work | Identity-key intro + help dialog (IdentityKeyIntroduction / IdentityKeyHelpDialog) | — | in-pr2 |
| `onboarding.key-intro.help-trigger` | What’s an identity key? | Identity-key intro + help dialog (IdentityKeyIntroduction / IdentityKeyHelpDialog) | — | in-pr2 |
| `onboarding.key-intro.help-title` | What’s an identity key? | Identity-key intro + help dialog (IdentityKeyIntroduction / IdentityKeyHelpDialog) | — | in-pr2 |
| `onboarding.key-intro.help-paragraph-1` | Buzz will create a Nostr identity with two parts: a private key that signs you in and a public key you can safely share. You can find your public identity anytime in Buzz settings. | Identity-key intro + help dialog (IdentityKeyIntroduction / IdentityKeyHelpDialog) | — | in-pr2 |
| `onboarding.key-intro.help-paragraph-2` | This identity belongs to you, not Buzz, and can move with you to another device or compatible Nostr app. Because only you control the private key, Buzz can't reset or recover it. Keep a backup somewhere safe, and never share it. | Identity-key intro + help dialog (IdentityKeyIntroduction / IdentityKeyHelpDialog) | — | in-pr2 |
| `onboarding.invite.opening-link` | Opening community link | Invite redemption (InviteRedeemForm / PendingInviteGate) | — | in-pr2 |
| `onboarding.invite.connect-after-setup` | You'll connect to {{communityName}} once setup is finished. | Invite redemption (InviteRedeemForm / PendingInviteGate) | communityName | in-pr2 |
| `onboarding.invite.continue-setup` | Continue setup | Invite redemption (InviteRedeemForm / PendingInviteGate) | — | in-pr2 |
| `onboarding.invite.cancel` | Cancel | Invite redemption (InviteRedeemForm / PendingInviteGate) | — | in-pr2 |
| `onboarding.invite.error-age-confirm` | Confirm that you are at least 18 years old. | Invite redemption (InviteRedeemForm / PendingInviteGate) | — | in-pr2 |
| `onboarding.invite.error-agree` | Agree to the Terms of Service and Privacy Policy. | Invite redemption (InviteRedeemForm / PendingInviteGate) | — | in-pr2 |
| `onboarding.invite.aria-redeeming` | Redeeming invite | Invite redemption (InviteRedeemForm / PendingInviteGate) | — | in-pr2 |
| `onboarding.invite.aria-loading-policy` | Loading policy | Invite redemption (InviteRedeemForm / PendingInviteGate) | — | in-pr2 |
| `onboarding.invite.next` | Next | Invite redemption (InviteRedeemForm / PendingInviteGate) | — | in-pr2 |
| `onboarding.invite.accept-join` | Accept and join | Invite redemption (InviteRedeemForm / PendingInviteGate) | — | in-pr2 |
| `onboarding.invite.join-community` | Join community | Invite redemption (InviteRedeemForm / PendingInviteGate) | — | in-pr2 |
| `onboarding.invite.accept-redeem` | Accept and redeem invite | Invite redemption (InviteRedeemForm / PendingInviteGate) | — | in-pr2 |
| `onboarding.invite.redeem` | Redeem invite | Invite redemption (InviteRedeemForm / PendingInviteGate) | — | in-pr2 |
| `onboarding.invite.back` | Back | Invite redemption (InviteRedeemForm / PendingInviteGate) | — | in-pr2 |
| `onboarding.invite.label-add-community` | Community URL or invite link | Invite redemption (InviteRedeemForm / PendingInviteGate) | — | in-pr2 |
| `onboarding.invite.label-invite` | Invite link or code | Invite redemption (InviteRedeemForm / PendingInviteGate) | — | in-pr2 |
| `onboarding.invite.relay-url-label` | Relay URL | Invite redemption (InviteRedeemForm / PendingInviteGate) | — | in-pr2 |
| `onboarding.invite.invalid-tip` | Please enter a valid invite link or community URL | Invite redemption (InviteRedeemForm / PendingInviteGate) | — | in-pr2 |
| `onboarding.membership.badge-required` | Membership required | Membership-denied + join-policy consent (MembershipDenied / JoinPolicyNotice / OnboardingFlow) | — | in-pr2 |
| `onboarding.membership.title` | Not a member yet | Membership-denied + join-policy consent (MembershipDenied / JoinPolicyNotice / OnboardingFlow) | — | in-pr2 |
| `onboarding.membership.body` | This relay requires an invitation. Ask a relay admin to add you as a member, then come back and try again. | Membership-denied + join-policy consent (MembershipDenied / JoinPolicyNotice / OnboardingFlow) | — | in-pr2 |
| `onboarding.membership.npub-label` | Your public key (npub) | Membership-denied + join-policy consent (MembershipDenied / JoinPolicyNotice / OnboardingFlow) | — | in-pr2 |
| `onboarding.membership.copy-npub` | Copy npub | Membership-denied + join-policy consent (MembershipDenied / JoinPolicyNotice / OnboardingFlow) | — | in-pr2 |
| `onboarding.membership.npub-hint` | This is your public identity — it's safe to share. Send it to the relay admin so they can invite you. | Membership-denied + join-policy consent (MembershipDenied / JoinPolicyNotice / OnboardingFlow) | — | in-pr2 |
| `onboarding.membership.unknown-pubkey` | Unknown public key | Membership-denied + join-policy consent (MembershipDenied / JoinPolicyNotice / OnboardingFlow) | — | in-pr2 |
| `onboarding.membership.error-import-key` | Failed to import key. | Membership-denied + join-policy consent (MembershipDenied / JoinPolicyNotice / OnboardingFlow) | — | in-pr2 |
| `onboarding.membership.import-key` | Import key | Membership-denied + join-policy consent (MembershipDenied / JoinPolicyNotice / OnboardingFlow) | — | in-pr2 |
| `onboarding.membership.back` | Back | Membership-denied + join-policy consent (MembershipDenied / JoinPolicyNotice / OnboardingFlow) | — | in-pr2 |
| `onboarding.membership.retry` | Try again | Membership-denied + join-policy consent (MembershipDenied / JoinPolicyNotice / OnboardingFlow) | — | in-pr2 |
| `onboarding.membership.change-community` | Change community | Membership-denied + join-policy consent (MembershipDenied / JoinPolicyNotice / OnboardingFlow) | — | in-pr2 |
| `onboarding.membership.have-invite` | Have an invite? | Membership-denied + join-policy consent (MembershipDenied / JoinPolicyNotice / OnboardingFlow) | — | in-pr2 |
| `onboarding.membership.use-different-key` | Use a different key | Membership-denied + join-policy consent (MembershipDenied / JoinPolicyNotice / OnboardingFlow) | — | in-pr2 |
| `onboarding.membership.error-server` | Server error — try again | Membership-denied + join-policy consent (MembershipDenied / JoinPolicyNotice / OnboardingFlow) | — | in-pr2 |
| `onboarding.membership.error-generic` | Something went wrong | Membership-denied + join-policy consent (MembershipDenied / JoinPolicyNotice / OnboardingFlow) | — | in-pr2 |
| `onboarding.membership.error-relay` | The relay returned an error. Try again. | Membership-denied + join-policy consent (MembershipDenied / JoinPolicyNotice / OnboardingFlow) | — | in-pr2 |
| `onboarding.membership.relay-unreachable-title` | Can't reach this relay | Membership-denied + join-policy consent (MembershipDenied / JoinPolicyNotice / OnboardingFlow) | — | in-pr2 |
| `onboarding.membership.relay-unreachable-body` | Check your connection or change your community. | Membership-denied + join-policy consent (MembershipDenied / JoinPolicyNotice / OnboardingFlow) | — | in-pr2 |
| `onboarding.membership.age-attestation` | I am 18 years of age or older. | Membership-denied + join-policy consent (MembershipDenied / JoinPolicyNotice / OnboardingFlow) | — | in-pr2 |
| `onboarding.membership.agree-intro` | I agree to the Buzz  | Membership-denied + join-policy consent (MembershipDenied / JoinPolicyNotice / OnboardingFlow) — sentence fragment, keeps exact DOM (no <Trans>) | — | in-pr2 |
| `onboarding.membership.terms-link` | Terms of Service | Membership-denied + join-policy consent (MembershipDenied / JoinPolicyNotice / OnboardingFlow) — sentence fragment, keeps exact DOM (no <Trans>) | — | in-pr2 |
| `onboarding.membership.agree-conjunction` |  and  | Membership-denied + join-policy consent (MembershipDenied / JoinPolicyNotice / OnboardingFlow) — sentence fragment, keeps exact DOM (no <Trans>) | — | in-pr2 |
| `onboarding.membership.privacy-link` | Privacy Policy | Membership-denied + join-policy consent (MembershipDenied / JoinPolicyNotice / OnboardingFlow) — sentence fragment, keeps exact DOM (no <Trans>) | — | in-pr2 |
| `onboarding.membership.agree-terminal` | . | Membership-denied + join-policy consent (MembershipDenied / JoinPolicyNotice / OnboardingFlow) — sentence fragment, keeps exact DOM (no <Trans>) | — | in-pr2 |
| `onboarding.community.aria-change-avatar` | Change your avatar | Community onboarding flow (CommunityOnboardingFlow) | — | in-pr2 |
| `onboarding.community.aria-add-avatar` | Add an avatar | Community onboarding flow (CommunityOnboardingFlow) | — | in-pr2 |
| `onboarding.community.joining` | Joining {{communityName}} | Community onboarding flow (CommunityOnboardingFlow) | communityName | in-pr2 |
| `onboarding.community.accepting-invite` | Accepting your invite… | Community onboarding flow (CommunityOnboardingFlow) | — | in-pr2 |
| `onboarding.community.connecting` | Connecting securely… | Community onboarding flow (CommunityOnboardingFlow) | — | in-pr2 |
| `onboarding.community.retry` | Retry | Community onboarding flow (CommunityOnboardingFlow) | — | in-pr2 |
| `onboarding.community.cancel` | Cancel | Community onboarding flow (CommunityOnboardingFlow) | — | in-pr2 |
| `onboarding.community.profile-title` | Build your profile | Community onboarding flow (CommunityOnboardingFlow) | — | in-pr2 |
| `onboarding.community.profile-body` | Add a name and avatar. They'll show up on your messages, reactions, and agent handoffs. | Community onboarding flow (CommunityOnboardingFlow) | — | in-pr2 |
| `onboarding.community.fallback-profile-name` | Your profile | Community onboarding flow (CommunityOnboardingFlow) | — | in-pr2 |
| `onboarding.community.username-label` | Your username | Community onboarding flow (CommunityOnboardingFlow) | — | in-pr2 |
| `onboarding.community.username-aria` | Community username | Community onboarding flow (CommunityOnboardingFlow) | — | in-pr2 |
| `onboarding.community.username-placeholder` | Enter your username here | Community onboarding flow (CommunityOnboardingFlow) | — | in-pr2 |
| `onboarding.community.next` | Next | Community onboarding flow (CommunityOnboardingFlow) | — | in-pr2 |
| `onboarding.community.edit-avatar-title` | Edit your avatar | Community onboarding flow (CommunityOnboardingFlow) | — | in-pr2 |
| `onboarding.community.team-title` | Meet your starter team | Community onboarding flow (CommunityOnboardingFlow) | — | in-pr2 |
| `onboarding.community.team-body` | Buzz lets you bring multiple agents into the same workspace. Your team will help you get started using Buzz. | Community onboarding flow (CommunityOnboardingFlow) | — | in-pr2 |
| `onboarding.community.persona-alt` | {{name}} animated character | Community onboarding flow (CommunityOnboardingFlow) | name | in-pr2 |
| `onboarding.community.error-retry-suffix` |  Try again. | Community onboarding flow (CommunityOnboardingFlow) | — | in-pr2 |
| `onboarding.community.preparing` | Preparing Welcome | Community onboarding flow (CommunityOnboardingFlow) | — | in-pr2 |
| `onboarding.community.enter` | Take me to Buzz | Community onboarding flow (CommunityOnboardingFlow) | — | in-pr2 |
| `onboarding.community.skip` | Skip for now | Community onboarding flow (CommunityOnboardingFlow) | — | in-pr2 |
| `onboarding.setup.connection-subscription` | Log in with a subscription | Harness + model settings (SetupStep / DefaultConfigStep / ConnectionMethodSection) | — | in-pr2 |
| `onboarding.setup.connection-api` | Use an API key | Harness + model settings (SetupStep / DefaultConfigStep / ConnectionMethodSection) | — | in-pr2 |
| `onboarding.setup.connect-provider-title` | Connect your AI provider | Harness + model settings (SetupStep / DefaultConfigStep / ConnectionMethodSection) | — | in-pr2 |
| `onboarding.setup.connect-provider-body` | Choose how your agents will access AI. You can change this later. | Harness + model settings (SetupStep / DefaultConfigStep / ConnectionMethodSection) | — | in-pr2 |
| `onboarding.setup.sign-in-required` | Sign in required | Harness + model settings (SetupStep / DefaultConfigStep / ConnectionMethodSection) | — | in-pr2 |
| `onboarding.setup.aria-sign-in` | Sign in to {{label}} | Harness + model settings (SetupStep / DefaultConfigStep / ConnectionMethodSection) | label | in-pr2 |
| `onboarding.setup.checking` | Checking… | Harness + model settings (SetupStep / DefaultConfigStep / ConnectionMethodSection) | — | in-pr2 |
| `onboarding.setup.check-again` | Check again | Harness + model settings (SetupStep / DefaultConfigStep / ConnectionMethodSection) | — | in-pr2 |
| `onboarding.setup.sign-in` | Sign in | Harness + model settings (SetupStep / DefaultConfigStep / ConnectionMethodSection) | — | in-pr2 |
| `onboarding.setup.error-signin-options` | Couldn't load sign-in options. | Harness + model settings (SetupStep / DefaultConfigStep / ConnectionMethodSection) | — | in-pr2 |
| `onboarding.setup.error-signin-unavailable` | Sign-in unavailable | Harness + model settings (SetupStep / DefaultConfigStep / ConnectionMethodSection) | — | in-pr2 |
| `onboarding.setup.error-signin-start` | Couldn't start sign-in. Try again. | Harness + model settings (SetupStep / DefaultConfigStep / ConnectionMethodSection) | — | in-pr2 |
| `onboarding.setup.error-signin-failed` | Sign-in failed | Harness + model settings (SetupStep / DefaultConfigStep / ConnectionMethodSection) | — | in-pr2 |
| `onboarding.setup.aria-installing` | Installing {{label}} | Harness + model settings (SetupStep / DefaultConfigStep / ConnectionMethodSection) | label | in-pr2 |
| `onboarding.setup.installing` | Installing | Harness + model settings (SetupStep / DefaultConfigStep / ConnectionMethodSection) | — | in-pr2 |
| `onboarding.setup.aria-check-again` | Check {{label}} again | Harness + model settings (SetupStep / DefaultConfigStep / ConnectionMethodSection) | label | in-pr2 |
| `onboarding.setup.retry-install` | Retry install | Harness + model settings (SetupStep / DefaultConfigStep / ConnectionMethodSection) | — | in-pr2 |
| `onboarding.setup.install` | Install | Harness + model settings (SetupStep / DefaultConfigStep / ConnectionMethodSection) | — | in-pr2 |
| `onboarding.setup.aria-retry-install` | Retry installing {{label}} | Harness + model settings (SetupStep / DefaultConfigStep / ConnectionMethodSection) | label | in-pr2 |
| `onboarding.setup.aria-install` | Install {{label}} | Harness + model settings (SetupStep / DefaultConfigStep / ConnectionMethodSection) | label | in-pr2 |
| `onboarding.setup.aria-view-instructions` | View {{label}} install instructions | Harness + model settings (SetupStep / DefaultConfigStep / ConnectionMethodSection) | label | in-pr2 |
| `onboarding.setup.detail-adapter-missing` | CLI detected; ACP adapter missing. | Harness + model settings (SetupStep / DefaultConfigStep / ConnectionMethodSection) | — | in-pr2 |
| `onboarding.setup.detail-adapter-outdated` | ACP adapter detected but outdated — reinstall required. | Harness + model settings (SetupStep / DefaultConfigStep / ConnectionMethodSection) | — | in-pr2 |
| `onboarding.setup.detail-cli-missing` | CLI not detected. | Harness + model settings (SetupStep / DefaultConfigStep / ConnectionMethodSection) | — | in-pr2 |
| `onboarding.setup.error-install-failed` | Install failed. | Harness + model settings (SetupStep / DefaultConfigStep / ConnectionMethodSection) | — | in-pr2 |
| `onboarding.setup.aria-open-setup` | Open {{label}} setup | Harness + model settings (SetupStep / DefaultConfigStep / ConnectionMethodSection) | label | in-pr2 |
| `onboarding.setup.recommended` | Recommended | Harness + model settings (SetupStep / DefaultConfigStep / ConnectionMethodSection) | — | in-pr2 |
| `onboarding.setup.tooltip-install-failed` | Installation failed | Harness + model settings (SetupStep / DefaultConfigStep / ConnectionMethodSection) | — | in-pr2 |
| `onboarding.setup.error-config-invalid-detail` | Check this runtime's configuration and try again. | Harness + model settings (SetupStep / DefaultConfigStep / ConnectionMethodSection) | — | in-pr2 |
| `onboarding.setup.error-config-invalid` | Configuration invalid | Harness + model settings (SetupStep / DefaultConfigStep / ConnectionMethodSection) | — | in-pr2 |
| `onboarding.setup.error-auth-verify` | Couldn't verify authentication. | Harness + model settings (SetupStep / DefaultConfigStep / ConnectionMethodSection) | — | in-pr2 |
| `onboarding.setup.error-status-unavailable` | Status unavailable | Harness + model settings (SetupStep / DefaultConfigStep / ConnectionMethodSection) | — | in-pr2 |
| `onboarding.setup.loading-providers` | Loading providers… | Harness + model settings (SetupStep / DefaultConfigStep / ConnectionMethodSection) | — | in-pr2 |
| `onboarding.setup.title-subscription` | Continue with an AI subscription | Harness + model settings (SetupStep / DefaultConfigStep / ConnectionMethodSection) | — | in-pr2 |
| `onboarding.setup.title-harness` | Choose a harness | Harness + model settings (SetupStep / DefaultConfigStep / ConnectionMethodSection) | — | in-pr2 |
| `onboarding.setup.body-subscription` | Subscriptions connect through a compatible harness, like Claude Code or Codex. Choose yours to sign in. | Harness + model settings (SetupStep / DefaultConfigStep / ConnectionMethodSection) | — | in-pr2 |
| `onboarding.setup.body-harness` | Choose how your agents will connect to AI providers. You can change this at any time. | Harness + model settings (SetupStep / DefaultConfigStep / ConnectionMethodSection) | — | in-pr2 |
| `onboarding.setup.not-installed` | Not installed | Harness + model settings (SetupStep / DefaultConfigStep / ConnectionMethodSection) | — | in-pr2 |
| `onboarding.setup.empty` | No supported harnesses are available for this connection method. | Harness + model settings (SetupStep / DefaultConfigStep / ConnectionMethodSection) | — | in-pr2 |
| `onboarding.setup.guide-set-up-title` | Set up {{label}} | Harness + model settings (SetupStep / DefaultConfigStep / ConnectionMethodSection) | label | in-pr2 |
| `onboarding.setup.guide-set-up-body` | Follow the setup guide to install {{label}}. When you're done, come back and check again. | Harness + model settings (SetupStep / DefaultConfigStep / ConnectionMethodSection) | label | in-pr2 |
| `onboarding.setup.open-guide` | Open guide | Harness + model settings (SetupStep / DefaultConfigStep / ConnectionMethodSection) | — | in-pr2 |
| `onboarding.setup.guide-connect-title` | Connect {{label}} | Harness + model settings (SetupStep / DefaultConfigStep / ConnectionMethodSection) | label | in-pr2 |
| `onboarding.setup.guide-connect-body` | Sign in to connect {{label}}. You can change this anytime. | Harness + model settings (SetupStep / DefaultConfigStep / ConnectionMethodSection) | label | in-pr2 |
| `onboarding.setup.skip-later` | Set up later | Harness + model settings (SetupStep / DefaultConfigStep / ConnectionMethodSection) | — | in-pr2 |
| `onboarding.setup.subscription-detail` | Buzz will open a sign-in window for {{label}}. | Harness + model settings (SetupStep / DefaultConfigStep / ConnectionMethodSection) | label | in-pr2 |
| `onboarding.setup.select-harness` | Select a harness | Harness + model settings (SetupStep / DefaultConfigStep / ConnectionMethodSection) | — | in-pr2 |
| `onboarding.setup.loading` | Loading… | Harness + model settings (SetupStep / DefaultConfigStep / ConnectionMethodSection) | — | in-pr2 |
| `onboarding.setup.error-load-harness` | Couldn't load harness settings. Go back and try again. | Harness + model settings (SetupStep / DefaultConfigStep / ConnectionMethodSection) | — | in-pr2 |
| `onboarding.setup.default-harness` | Default harness | Harness + model settings (SetupStep / DefaultConfigStep / ConnectionMethodSection) | — | in-pr2 |
| `onboarding.setup.or` | or | Harness + model settings (SetupStep / DefaultConfigStep / ConnectionMethodSection) | — | in-pr2 |
| `onboarding.setup.use-different-harness` | Use a different harness | Harness + model settings (SetupStep / DefaultConfigStep / ConnectionMethodSection) | — | in-pr2 |
| `onboarding.setup.title-api-key` | Connect with an API key | Harness + model settings (SetupStep / DefaultConfigStep / ConnectionMethodSection) | — | in-pr2 |
| `onboarding.setup.title-model-settings` | Choose your model settings | Harness + model settings (SetupStep / DefaultConfigStep / ConnectionMethodSection) | — | in-pr2 |
| `onboarding.setup.body-api-key` | Choose your provider and enter an API key to connect to the Buzz harness. | Harness + model settings (SetupStep / DefaultConfigStep / ConnectionMethodSection) | — | in-pr2 |
| `onboarding.setup.body-model-settings` | Select the model and effort level your agents will use by default. | Harness + model settings (SetupStep / DefaultConfigStep / ConnectionMethodSection) | — | in-pr2 |
| `onboarding.setup.saving` | Saving… | Harness + model settings (SetupStep / DefaultConfigStep / ConnectionMethodSection) | — | in-pr2 |
| `onboarding.setup.next` | Next | Harness + model settings (SetupStep / DefaultConfigStep / ConnectionMethodSection) | — | in-pr2 |
| `onboarding.setup.error-save-model-generic` | Couldn't save model settings. | Harness + model settings (SetupStep / DefaultConfigStep / ConnectionMethodSection) | — | in-pr2 |
| `onboarding.setup.error-save-model` | Couldn't save model settings. {{error}} Try again. | Harness + model settings (SetupStep / DefaultConfigStep / ConnectionMethodSection) | error | in-pr2 |
| `onboarding.setup.skip-now` | Skip for now | Harness + model settings (SetupStep / DefaultConfigStep / ConnectionMethodSection) | — | in-pr2 |
| `onboarding.starter-channels.error-setup` | Couldn't set up starter channels | Starter-channel failure toast (onboarding/hooks.ts) | — | in-pr2 |
| `onboarding.starter-channels.retry` | Retry | Starter-channel failure toast (onboarding/hooks.ts) | — | in-pr2 |
| `onboarding.starter-channels.fallback-reason` | Failed to set up starter channels | Starter-channel failure toast (onboarding/hooks.ts) — only when the failure carries no message; a raw failure message is shown verbatim | — | in-pr2 |
| `sidebar.nav.projects` | Projects | Pinned nav header (AppSidebarPinnedHeader) | — | in-pr3 |
| `sidebar.nav.inbox` | Inbox | Pinned nav header (AppSidebarPinnedHeader) | — | in-pr3 |
| `sidebar.nav.pulse` | Pulse | Pinned nav header (AppSidebarPinnedHeader) | — | in-pr3 |
| `sidebar.nav.agents` | Agents | Pinned nav header (AppSidebarPinnedHeader) | — | in-pr3 |
| `sidebar.nav.workflows` | Workflows | Pinned nav header (AppSidebarPinnedHeader) | — | in-pr3 |
| `sidebar.shell.channels` | Channels | Sidebar shell + section list (AppSidebar / SidebarSection / CustomChannelSection) | — | in-pr3 |
| `sidebar.shell.starred` | Starred | Sidebar shell + section list (AppSidebar / SidebarSection / CustomChannelSection) | — | in-pr3 |
| `sidebar.shell.forums` | Forums | Sidebar shell + section list (AppSidebar / SidebarSection / CustomChannelSection) | — | in-pr3 |
| `sidebar.shell.new-message` | New message | Sidebar shell + section list (AppSidebar / SidebarSection / CustomChannelSection) | — | in-pr3 |
| `sidebar.shell.direct-messages` | Direct messages | Sidebar shell + section list (AppSidebar / SidebarSection / CustomChannelSection) | — | in-pr3 |
| `sidebar.shell.close-direct-message` | Close direct message | Sidebar shell + section list (AppSidebar / SidebarSection / CustomChannelSection) | — | in-pr3 |
| `sidebar.rail.title` | Communities | Community rail (CommunityRail) | — | in-pr3 |
| `sidebar.rail.add` | Add community | Community rail (CommunityRail) | — | in-pr3 |
| `sidebar.activity.title` | Channel activity | Channel activity popover (ChannelActivityPopover) | — | in-pr3 |
| `sidebar.activity.thread` | Thread | Channel activity popover (ChannelActivityPopover) | — | in-pr3 |
| `sidebar.activity.remind-later` | Remind me later | Channel activity popover (ChannelActivityPopover) | — | in-pr3 |
| `sidebar.channel.mark-read` | Mark as read | Channel context menu (ChannelContextMenu / ChannelActivityPopover) | — | in-pr3 |
| `sidebar.channel.mark-unread` | Mark unread | Channel context menu (ChannelContextMenu / ChannelActivityPopover) | — | in-pr3 |
| `sidebar.channel.mute` | Mute channel | Channel context menu (ChannelContextMenu / ChannelActivityPopover) | — | in-pr3 |
| `sidebar.channel.unmute` | Unmute channel | Channel context menu (ChannelContextMenu / ChannelActivityPopover) | — | in-pr3 |
| `sidebar.channel.star` | Star channel | Channel context menu (ChannelContextMenu / ChannelActivityPopover) | — | in-pr3 |
| `sidebar.channel.unstar` | Unstar channel | Channel context menu (ChannelContextMenu / ChannelActivityPopover) | — | in-pr3 |
| `sidebar.channel.leave` | Leave channel | Channel context menu (ChannelContextMenu / ChannelActivityPopover) | — | in-pr3 |
| `sidebar.channel.archive` | Archive channel | Channel context menu (ChannelContextMenu / ChannelActivityPopover) | — | in-pr3 |
| `sidebar.channel.delete` | Delete channel | Channel context menu (ChannelContextMenu / ChannelActivityPopover) | — | in-pr3 |
| `sidebar.channel.copy` | Copy | Channel context menu (ChannelContextMenu / ChannelActivityPopover) | — | in-pr3 |
| `sidebar.channel.copy-name` | Copy channel name | Channel context menu (ChannelContextMenu / ChannelActivityPopover) | — | in-pr3 |
| `sidebar.channel.copy-id` | Copy channel ID | Channel context menu (ChannelContextMenu / ChannelActivityPopover) | — | in-pr3 |
| `sidebar.channel.copied-name` | Channel name copied to clipboard | Channel context menu (ChannelContextMenu / ChannelActivityPopover) | — | in-pr3 |
| `sidebar.channel.copied-id` | Channel ID copied to clipboard | Channel context menu (ChannelContextMenu / ChannelActivityPopover) | — | in-pr3 |
| `sidebar.channel.loading-actions` | Loading channel actions... | Channel context menu (ChannelContextMenu / ChannelActivityPopover) | — | in-pr3 |
| `sidebar.channel.actions-unavailable` | Channel actions unavailable | Channel context menu (ChannelContextMenu / ChannelActivityPopover) | — | in-pr3 |
| `sidebar.sections.move-to` | Move to section | Channel sections (ChannelSectionDialogs / CustomChannelSection / ChannelContextMenu) | — | in-pr3 |
| `sidebar.sections.new` | New section... | Channel sections (ChannelSectionDialogs / CustomChannelSection / ChannelContextMenu) | — | in-pr3 |
| `sidebar.sections.remove-from` | Remove from section | Channel sections (ChannelSectionDialogs / CustomChannelSection / ChannelContextMenu) | — | in-pr3 |
| `sidebar.sections.create` | Create section | Channel sections (ChannelSectionDialogs / CustomChannelSection / ChannelContextMenu) | — | in-pr3 |
| `sidebar.sections.rename` | Rename section | Channel sections (ChannelSectionDialogs / CustomChannelSection / ChannelContextMenu) | — | in-pr3 |
| `sidebar.sections.delete` | Delete section | Channel sections (ChannelSectionDialogs / CustomChannelSection / ChannelContextMenu) | — | in-pr3 |
| `sidebar.sections.name` | Section name | Channel sections (ChannelSectionDialogs / CustomChannelSection / ChannelContextMenu) | — | in-pr3 |
| `sidebar.sections.choose-icon` | Choose section icon | Channel sections (ChannelSectionDialogs / CustomChannelSection / ChannelContextMenu) | — | in-pr3 |
| `sidebar.sections.clear-icon` | Clear section icon | Channel sections (ChannelSectionDialogs / CustomChannelSection / ChannelContextMenu) | — | in-pr3 |
| `sidebar.sections.mark-all-read` | Mark all as read | Channel sections (ChannelSectionDialogs / CustomChannelSection / ChannelContextMenu) | — | in-pr3 |
| `sidebar.sections.move-up` | Move up | Channel sections (ChannelSectionDialogs / CustomChannelSection / ChannelContextMenu) | — | in-pr3 |
| `sidebar.sections.move-down` | Move down | Channel sections (ChannelSectionDialogs / CustomChannelSection / ChannelContextMenu) | — | in-pr3 |
| `sidebar.sections.recent` | Recent | Channel sections (ChannelSectionDialogs / CustomChannelSection / ChannelContextMenu) | — | in-pr3 |
| `sidebar.projects.add` | Add project | Projects section (SidebarProjectsSection) | — | in-pr3 |
| `sidebar.projects.more-actions` | More actions for Projects | Projects section (SidebarProjectsSection) | — | in-pr3 |
| `sidebar.projects.show` | Show | Projects section (SidebarProjectsSection) | — | in-pr3 |
| `sidebar.projects.browse-all` | Browse all projects | Projects section (SidebarProjectsSection) | — | in-pr3 |
| `sidebar.projects.remove-from-sidebar` | Remove from sidebar | Projects section (SidebarProjectsSection) | — | in-pr3 |
| `sidebar.projects.copy-link` | Copy link | Projects section (SidebarProjectsSection) | — | in-pr3 |
| `sidebar.projects.copied-link` | Link copied to clipboard | Projects section (SidebarProjectsSection) | — | in-pr3 |
| `sidebar.projects.delete` | Delete project | Projects section (SidebarProjectsSection) | — | in-pr3 |
| `sidebar.projects.delete-confirm` | Delete project? | Projects section (SidebarProjectsSection) | — | in-pr3 |
| `sidebar.projects.deleted` | Project deleted | Projects section (SidebarProjectsSection) | — | in-pr3 |
| `sidebar.common.cancel` | Cancel | Shared sidebar labels | — | in-pr3 |
| `sidebar.common.sort` | Sort | Shared sidebar labels | — | in-pr3 |
| `sidebar.channel-form.type` | Type | Create-channel form fields (CreateChannelFormFields) | — | in-pr3 |
| `sidebar.channel-form.private` | Private | Create-channel form fields (CreateChannelFormFields) | — | in-pr3 |
| `sidebar.channel-form.optional` | Optional | Create-channel form fields (CreateChannelFormFields) | — | in-pr3 |
| `sidebar.shell.unread-count_one` | unread notification | Sidebar shell + section list (AppSidebar / SidebarSection / CustomChannelSection) | — | in-pr3 |
| `sidebar.shell.unread-count_other` | unread notifications | Sidebar shell + section list (AppSidebar / SidebarSection / CustomChannelSection) | — | in-pr3 |
| `sidebar.rail.unread-tooltip` | {{community}} — unread | Community rail (CommunityRail) | — | in-pr3 |
| `sidebar.rail.mention-count_one` | {{community}} — {{count}} mention | Community rail (CommunityRail) | — | in-pr3 |
| `sidebar.rail.mention-count_other` | {{community}} — {{count}} mentions | Community rail (CommunityRail) | — | in-pr3 |
| `sidebar.channel.create` | Create channel | Channel context menu (ChannelContextMenu / ChannelActivityPopover / CustomChannelSection) | — | in-pr3 |
| `sidebar.channel.browse` | Browse channels | Channel context menu (ChannelContextMenu / ChannelActivityPopover / CustomChannelSection) | — | in-pr3 |
| `sidebar.sections.alpha` | A–Z | Channel sections (ChannelSectionDialogs / CustomChannelSection / ChannelContextMenu) | — | in-pr3 |
| `sidebar.sections.delete-body-empty` | Delete section "{{sectionName}}"? It has no channels. | Channel sections (ChannelSectionDialogs / CustomChannelSection / ChannelContextMenu) | — | in-pr3 |
| `sidebar.sections.delete-body` | Delete section "{{sectionName}}"? Its {{channelLabel}} will move back to the default Channels group. | Channel sections (ChannelSectionDialogs / CustomChannelSection / ChannelContextMenu) | — | in-pr3 |
| `sidebar.sections.channel-count_one` | {{count}} channel | Channel sections (ChannelSectionDialogs / CustomChannelSection / ChannelContextMenu) | — | in-pr3 |
| `sidebar.sections.channel-count_other` | {{count}} channels | Channel sections (ChannelSectionDialogs / CustomChannelSection / ChannelContextMenu) | — | in-pr3 |
| `sidebar.projects.created` | Project "{{name}}" created. | Projects section (SidebarProjectsSection) | — | in-pr3 |
| `sidebar.channel-form.open` | Open | Create-channel form fields (CreateChannelFormFields / CreateChannelDialog) | — | in-pr3 |
| `sidebar.channel-form.template` | Template | Create-channel form fields (CreateChannelFormFields / CreateChannelDialog) | — | in-pr3 |
| `sidebar.channel-form.none` | None | Create-channel form fields (CreateChannelFormFields / CreateChannelDialog) | — | in-pr3 |
| `sidebar.channel-form.canvas-included` | Canvas included | Create-channel form fields (CreateChannelFormFields / CreateChannelDialog) — template summary fragment | — | in-pr3 |
| `sidebar.channel-form.team-count_one` | {{count}} team | Create-channel form fields (CreateChannelFormFields / CreateChannelDialog) | — | in-pr3 |
| `sidebar.channel-form.team-count_other` | {{count}} teams | Create-channel form fields (CreateChannelFormFields / CreateChannelDialog) | — | in-pr3 |
| `sidebar.channel-form.agent-count_one` | {{count}} agent | Create-channel form fields (CreateChannelFormFields / CreateChannelDialog) | — | in-pr3 |
| `sidebar.channel-form.agent-count_other` | {{count}} agents | Create-channel form fields (CreateChannelFormFields / CreateChannelDialog) | — | in-pr3 |
| `sidebar.relay.waiting-reconnect` | Waiting to reconnect | Relay connection card (SidebarRelayConnectionCard) | — | in-pr3 |
| `sidebar.relay.connecting` | Connecting | Relay connection card (SidebarRelayConnectionCard) | — | in-pr3 |
| `sidebar.relay.reconnecting` | Reconnecting | Relay connection card (SidebarRelayConnectionCard) | — | in-pr3 |
| `sidebar.relay.connected` | Connected | Relay connection card (SidebarRelayConnectionCard) | — | in-pr3 |
| `sidebar.relay.connect-aria` | Connect to relay | Relay connection card (SidebarRelayConnectionCard) — accessible name; the visible label is separate | — | in-pr3 |
| `sidebar.relay.connect-hint` | Click to connect | Relay connection card (SidebarRelayConnectionCard) | — | in-pr3 |
| `sidebar.relay.unreachable` | Can't reach the relay | Relay connection card (SidebarRelayConnectionCard) | — | in-pr3 |
| `sidebar.relay.reconnect-help` | Complete any prompts opened by the reconnect helper to continue. | Relay connection card (SidebarRelayConnectionCard) | — | in-pr3 |
| `sidebar.profile-card.no-community` | No community | Sidebar profile card (SidebarProfileCard) | — | in-pr3 |
| `sidebar.channel-dialog.description-forums` | Forums organize threaded discussions around a topic. | undefined | — | in-pr3 |
| `sidebar.channel-dialog.description-channels` | Channels are real-time streams for team conversation. | undefined | — | in-pr3 |
| `sidebar.channel-dialog.template-aria` | Template: {{template}} | undefined | — | in-pr3 |
| `sidebar.projects.created-standalone` | Created as a standalone project | Projects section (SidebarProjectsSection) | — | in-pr3 |
| `sidebar.projects.delete-failed` | Failed to delete project | Projects section (SidebarProjectsSection) | — | in-pr3 |

## Deliberately untranslated (do not add rows)

- Brand names (`Buzz`), language endonyms shown to the user as-is.
- The appearance “Preferences” card title and the density / link-preview /
  thread-layout rows are still PR-4 surfaces; they stay English until that
  PR extracts them.

| `channels.activity.agent-name` | Agent | BotActivityBar | — | in-pr3 |
| `channels.activity.agent-working` | {{name}} is working | BotActivityBar | — | in-pr3 |
| `channels.activity.agents-working` | {{total}} agents working | BotActivityBar | — | in-pr3 |
| `channels.activity.agents-working-header` | Agents working | BotActivityBar | — | in-pr3 |
| `channels.activity.ago-days` | {{count}}d ago | AgentSessionThreadPanel | — | in-pr3 |
| `channels.activity.ago-hours` | {{count}}h ago | AgentSessionThreadPanel | — | in-pr3 |
| `channels.activity.ago-minutes` | {{count}}m ago | AgentSessionThreadPanel | — | in-pr3 |
| `channels.activity.ago-weeks` | {{count}}w ago | AgentSessionThreadPanel | — | in-pr3 |
| `channels.activity.animations-label` | Show Animations | AgentSessionThreadPanel | — | in-pr3 |
| `channels.activity.animations-title-off` | Animate new activity rows as they arrive. | AgentSessionThreadPanel | — | in-pr3 |
| `channels.activity.animations-title-on` | Stop animating new activity rows. | AgentSessionThreadPanel | — | in-pr3 |
| `channels.activity.animations-title-raw` | Raw activity rows don't animate in. | AgentSessionThreadPanel | — | in-pr3 |
| `channels.activity.back-aria` | Back from activity | AgentSessionThreadPanel | — | in-pr3 |
| `channels.activity.empty-any` | Mention {{name}} in any channel to see its work here. | AgentSessionThreadPanel | — | in-pr3 |
| `channels.activity.empty-scoped` | Mention {{name}} in the channel to see its work here. | AgentSessionThreadPanel | — | in-pr3 |
| `channels.activity.just-now` | just now | AgentSessionThreadPanel | — | in-pr3 |
| `channels.activity.last-updated` | Last updated {{time}} | AgentSessionThreadPanel | — | in-pr3 |
| `channels.activity.last-updated-title` | Last updated {{timestamp}} | AgentSessionThreadPanel | — | in-pr3 |
| `channels.activity.no-updates` | No updates yet | AgentSessionThreadPanel | — | in-pr3 |
| `channels.activity.raw-description` | Show raw JSON-RPC activity. | AgentSessionThreadPanel | — | in-pr3 |
| `channels.activity.raw-label` | Raw | AgentSessionThreadPanel | — | in-pr3 |
| `channels.activity.raw-title-agent` | Show raw JSON-RPC payloads for this agent. | AgentSessionThreadPanel | — | in-pr3 |
| `channels.activity.raw-title-channel` | Show raw JSON-RPC payloads for this channel. | AgentSessionThreadPanel | — | in-pr3 |
| `channels.activity.raw-title-hide` | Hide raw JSON-RPC payloads. | AgentSessionThreadPanel | — | in-pr3 |
| `channels.activity.scope-all-channels` | All channels | AgentSessionThreadPanel | — | in-pr3 |
| `channels.activity.scope-channel-count_one` | {{count}} channel | AgentSessionThreadPanel | — | in-pr3 |
| `channels.activity.scope-channel-count_other` | {{count}} channels | AgentSessionThreadPanel | — | in-pr3 |
| `channels.activity.scope-title` | {{view}} · {{scope}} | AgentSessionThreadPanel | — | in-pr3 |
| `channels.activity.settings-aria` | Open activity settings | AgentSessionThreadPanel | — | in-pr3 |
| `channels.activity.settings-title` | Activity settings | AgentSessionThreadPanel | — | in-pr3 |
| `channels.activity.stop-body-not-local` | Only available for locally managed agents. | AgentSessionThreadPanel | — | in-pr3 |
| `channels.activity.stop-error-failed` | Failed to stop {{name}}'s current turn. | AgentSessionThreadPanel | — | in-pr3 |
| `channels.activity.stop-error-multi-session` | This channel has multiple agent sessions. Stopping a specific thread isn't available here yet. | AgentSessionThreadPanel | — | in-pr3 |
| `channels.activity.stop-hint-no-channel` | Open activity for a channel to stop its current turn. | AgentSessionThreadPanel | — | in-pr3 |
| `channels.activity.stop-info-no-active-turn` | No active turn to stop. | AgentSessionThreadPanel | — | in-pr3 |
| `channels.activity.stop-info-unconfirmed` | Stop requested, but the agent hasn't confirmed it. | AgentSessionThreadPanel | — | in-pr3 |
| `channels.activity.stop-label` | Stop current turn | AgentSessionThreadPanel | — | in-pr3 |
| `channels.activity.stop-success-sent` | Stop signal sent to {{name}}. It may take a moment to respond. | AgentSessionThreadPanel | — | in-pr3 |
| `channels.activity.stop-title-available` | Interrupt the current ACP turn without stopping the agent process. | AgentSessionThreadPanel | — | in-pr3 |
| `channels.activity.stop-title-idle` | Available while the agent is working. | AgentSessionThreadPanel | — | in-pr3 |
| `channels.activity.stop-title-not-local` | Only locally managed agents can be interrupted from this community. | AgentSessionThreadPanel | — | in-pr3 |
| `channels.activity.timestamps-label` | Show Timestamps | AgentSessionThreadPanel | — | in-pr3 |
| `channels.activity.timestamps-title-off` | Show a timestamp under each activity row. | AgentSessionThreadPanel | — | in-pr3 |
| `channels.activity.timestamps-title-on` | Hide per-row activity timestamps. | AgentSessionThreadPanel | — | in-pr3 |
| `channels.activity.view-activity` | View activity | BotActivityBar | — | in-pr3 |
| `channels.activity.view-activity-aria` | {{label}}. View activity. | BotActivityBar | — | in-pr3 |
| `channels.activity.view-activity-label` | Activity | AgentSessionThreadPanel | — | in-pr3 |
| `channels.activity.view-raw` | Raw ACP activity | AgentSessionThreadPanel | — | in-pr3 |
| `channels.activity.working-sr` | working | BotActivityBar | — | in-pr3 |
| `channels.activity.working-status` | Working | BotActivityBar | — | in-pr3 |
| `channels.bot.add-agent` | Add agent | AddChannelBotDialog | — | in-pr3 |
| `channels.bot.add-agents-count` | Add {{total}} agents | AddChannelBotDialog | — | in-pr3 |
| `channels.bot.add-failed` | Failed to add {{name}}: {{error}} | AddChannelBotDialog | — | in-pr3 |
| `channels.bot.add-instance` | Add {{instance}} ({{name}}) | QuickBotBar | — | in-pr3 |
| `channels.bot.add-named` | Add {{name}} | QuickBotBar | — | in-pr3 |
| `channels.bot.added-agents_one` | Added {{count}} agent. | AddChannelBotDialog | — | in-pr3 |
| `channels.bot.added-agents_other` | Added {{count}} agents. | AddChannelBotDialog | — | in-pr3 |
| `channels.bot.adding` | Adding… | AddChannelBotDialog | — | in-pr3 |
| `channels.bot.adding-count` | Adding {{total}}… | AddChannelBotDialog | — | in-pr3 |
| `channels.bot.all-agents-in-channel` | All of your agents are already in this channel. | AddChannelBotPersonasSection | — | in-pr3 |
| `channels.bot.all-in-channel` | All in channel | AddChannelBotTeamsSection | — | in-pr3 |
| `channels.bot.create-failed` | Failed to create agent. | useQuickBotDrop | — | in-pr3 |
| `channels.bot.create-new` | Create a new agent | AddChannelBotPersonasSection | — | in-pr3 |
| `channels.bot.create-new-hint` | Give it a name, purpose, and instructions. | AddChannelBotPersonasSection | — | in-pr3 |
| `channels.bot.dialog-description` | Choose from your agents, or create a new one. | AddChannelBotDialog | — | in-pr3 |
| `channels.bot.generic-agent` | Generic agent | AddChannelBotGenericSection | — | in-pr3 |
| `channels.bot.generic-description` | Add one custom agent alongside any selected agents. | AddChannelBotGenericSection | — | in-pr3 |
| `channels.bot.in-channel` | In channel | AddChannelBotPersonasSection | — | in-pr3 |
| `channels.bot.in-channel-count` | {{total}} in channel | AddChannelBotTeamsSection | — | in-pr3 |
| `channels.bot.in-this-channel` | In this channel | AddChannelBotPersonasSection | — | in-pr3 |
| `channels.bot.instance-label` | Agent instance | AddChannelBotReuseGuard | — | in-pr3 |
| `channels.bot.loading` | Loading your agents… | AddChannelBotPersonasSection | — | in-pr3 |
| `channels.bot.name` | Name | AddChannelBotGenericSection | — | in-pr3 |
| `channels.bot.name-hint` | Defaults to the selected runtime name. | AddChannelBotGenericSection | — | in-pr3 |
| `channels.bot.no-runtime` | No agent runtime available. | useQuickBotDrop | — | in-pr3 |
| `channels.bot.no-runtime-warning` | Install an agent runtime before adding an agent to this channel. | AddChannelBotDialog | — | in-pr3 |
| `channels.bot.option-new` | Create new instance | AddChannelBotReuseGuard | — | in-pr3 |
| `channels.bot.option-reuse` | Reuse existing agent | AddChannelBotReuseGuard | — | in-pr3 |
| `channels.bot.prompt` | Prompt | AddChannelBotGenericSection | — | in-pr3 |
| `channels.bot.prompt-hint` | Saved as the generic agent's system prompt override. | AddChannelBotGenericSection | — | in-pr3 |
| `channels.bot.prompt-placeholder` | What should this agent help with in the channel? | AddChannelBotGenericSection | — | in-pr3 |
| `channels.bot.quick-add-toolbar` | Quick add bots | QuickBotBar | — | in-pr3 |
| `channels.bot.reuse-note` | {{name}} is already {{status}}. Reusing adds it to this channel without creating a duplicate keypair. | AddChannelBotReuseGuard | — | in-pr3 |
| `channels.bot.status-running` | running | AddChannelBotReuseGuard | — | in-pr3 |
| `channels.bot.status-stopped` | stopped | AddChannelBotReuseGuard | — | in-pr3 |
| `channels.bot.teams` | Teams | AddChannelBotTeamsSection | — | in-pr3 |
| `channels.bot.teams-hint` | Select a team to toggle all its agents at once. | AddChannelBotTeamsSection | — | in-pr3 |
| `channels.bot.title` | Add agents | AddChannelBotDialog | — | in-pr3 |
| `channels.bot.your-agents` | Your agents | AddChannelBotPersonasSection | — | in-pr3 |
| `channels.browser.back-to-search` | Back to search | ChannelBrowserDialog | — | in-pr3 |
| `channels.browser.badge-archived` | archived | ChannelBrowserDialog | — | in-pr3 |
| `channels.browser.close` | Close | ChannelBrowserDialog | — | in-pr3 |
| `channels.browser.create-new-channel` | Create a new channel | ChannelBrowserDialog | — | in-pr3 |
| `channels.browser.create-new-forum` | Create a new forum | ChannelBrowserDialog | — | in-pr3 |
| `channels.browser.create-prefix-channel` | Create channel  | ChannelBrowserDialog | — | in-pr3 |
| `channels.browser.create-prefix-forum` | Create forum  | ChannelBrowserDialog | — | in-pr3 |
| `channels.browser.create-query` | “{{query}}” | ChannelBrowserDialog | — | in-pr3 |
| `channels.browser.create-title-channel` | New channel | ChannelBrowserDialog | — | in-pr3 |
| `channels.browser.create-title-forum` | New forum | ChannelBrowserDialog | — | in-pr3 |
| `channels.browser.empty-archived-channel` | No archived channels | ChannelBrowserDialog | — | in-pr3 |
| `channels.browser.empty-archived-forum` | No archived forums | ChannelBrowserDialog | — | in-pr3 |
| `channels.browser.empty-archived-note-channel` | Archived channels you have joined will appear here. | ChannelBrowserDialog | — | in-pr3 |
| `channels.browser.empty-archived-note-forum` | Archived forums you have joined will appear here. | ChannelBrowserDialog | — | in-pr3 |
| `channels.browser.empty-create-note-channel` | No channel by that name yet — create it to get started. | ChannelBrowserDialog | — | in-pr3 |
| `channels.browser.empty-create-note-forum` | No forum by that name yet — create it to get started. | ChannelBrowserDialog | — | in-pr3 |
| `channels.browser.empty-joined-channel` | No joined channels | ChannelBrowserDialog | — | in-pr3 |
| `channels.browser.empty-joined-forum` | No joined forums | ChannelBrowserDialog | — | in-pr3 |
| `channels.browser.empty-joined-note-channel` | Channels you join will appear here. | ChannelBrowserDialog | — | in-pr3 |
| `channels.browser.empty-joined-note-forum` | Forums you join will appear here. | ChannelBrowserDialog | — | in-pr3 |
| `channels.browser.empty-no-results-channel` | No channels match your search | ChannelBrowserDialog | — | in-pr3 |
| `channels.browser.empty-no-results-forum` | No forums match your search | ChannelBrowserDialog | — | in-pr3 |
| `channels.browser.empty-none-channel` | No channels to browse | ChannelBrowserDialog | — | in-pr3 |
| `channels.browser.empty-none-forum` | No forums to browse | ChannelBrowserDialog | — | in-pr3 |
| `channels.browser.empty-none-note-channel` | All open channels are available in the sidebar. Create a new channel to get started. | ChannelBrowserDialog | — | in-pr3 |
| `channels.browser.empty-none-note-forum` | All open forums are available in the sidebar. Create a new forum to get started. | ChannelBrowserDialog | — | in-pr3 |
| `channels.browser.empty-select-channel` | Select a channel to view messages. | ChannelScreenEmptyState | — | in-pr3 |
| `channels.browser.empty-try-other` | Try a different name or keyword. | ChannelBrowserDialog | — | in-pr3 |
| `channels.browser.join` | Join | ChannelBrowserDialog | — | in-pr3 |
| `channels.browser.joining` | Joining... | ChannelBrowserDialog | — | in-pr3 |
| `channels.browser.member-count_one` | {{count}} member | ChannelBrowserDialog | — | in-pr3 |
| `channels.browser.member-count_other` | {{count}} members | ChannelBrowserDialog | — | in-pr3 |
| `channels.browser.search-channel` | Search channels by name or description | ChannelBrowserDialog | — | in-pr3 |
| `channels.browser.search-create-channel` | Search or create a channel | ChannelBrowserDialog | — | in-pr3 |
| `channels.browser.search-create-forum` | Search or create a forum | ChannelBrowserDialog | — | in-pr3 |
| `channels.browser.search-forum` | Search forums by name or description | ChannelBrowserDialog | — | in-pr3 |
| `channels.browser.sort-alpha` | Alphabetical | ChannelBrowserDialog | — | in-pr3 |
| `channels.browser.sort-aria-channel` | Sort channels: {{label}} | ChannelBrowserDialog | — | in-pr3 |
| `channels.browser.sort-aria-forum` | Sort forums: {{label}} | ChannelBrowserDialog | — | in-pr3 |
| `channels.browser.sort-by` | Sort by | ChannelBrowserDialog | — | in-pr3 |
| `channels.browser.sort-members` | Most members | ChannelBrowserDialog | — | in-pr3 |
| `channels.browser.sort-recent` | Recent | ChannelBrowserDialog | — | in-pr3 |
| `channels.browser.tab-all-channel` | All channels | ChannelBrowserDialog | — | in-pr3 |
| `channels.browser.tab-all-forum` | All forums | ChannelBrowserDialog | — | in-pr3 |
| `channels.browser.tab-archived` | Archived | ChannelBrowserDialog | — | in-pr3 |
| `channels.browser.tab-joined` | Joined | ChannelBrowserDialog | — | in-pr3 |
| `channels.browser.title-channel` | Browse channels | ChannelBrowserDialog | — | in-pr3 |
| `channels.browser.title-forum` | Add a forum | ChannelBrowserDialog | — | in-pr3 |
| `channels.canvas.cancel` | Cancel | ChannelCanvas | — | in-pr3 |
| `channels.canvas.content-aria` | Canvas content | ChannelCanvas | — | in-pr3 |
| `channels.canvas.create` | Create canvas | ChannelCanvas | — | in-pr3 |
| `channels.canvas.edit` | Edit canvas | ChannelCanvas | — | in-pr3 |
| `channels.canvas.empty` | No canvas set for this channel. | ChannelCanvas | — | in-pr3 |
| `channels.canvas.loading` | Loading canvas... | ChannelCanvas | — | in-pr3 |
| `channels.canvas.placeholder` | Write your canvas content in Markdown... | ChannelCanvas | — | in-pr3 |
| `channels.canvas.save` | Save canvas | ChannelCanvas | — | in-pr3 |
| `channels.canvas.saving` | Saving... | ChannelCanvas | — | in-pr3 |
| `channels.description.archived` | Archived. | channelDescription | — | in-pr3 |
| `channels.description.details` | Channel details and activity. | channelDescription | — | in-pr3 |
| `channels.description.no-relay` | Connect to the relay to browse channels and read messages. | channelDescription | — | in-pr3 |
| `channels.description.read-only-until-join` | Read-only until you join this open channel. | channelDescription | — | in-pr3 |
| `channels.dm.more-count` | +{{total}} more | dmParticipantDisplay | — | in-pr3 |
| `channels.ephemeral.cleanup-due` | Cleanup due | ephemeralChannel | — | in-pr3 |
| `channels.ephemeral.duration-days_one` | {{count}} day | ephemeralChannel | — | in-pr3 |
| `channels.ephemeral.duration-days_other` | {{count}} days | ephemeralChannel | — | in-pr3 |
| `channels.ephemeral.duration-hours_one` | {{count}} hour | ephemeralChannel | — | in-pr3 |
| `channels.ephemeral.duration-hours_other` | {{count}} hours | ephemeralChannel | — | in-pr3 |
| `channels.ephemeral.duration-minutes_one` | {{count}} minute | ephemeralChannel | — | in-pr3 |
| `channels.ephemeral.duration-minutes_other` | {{count}} minutes | ephemeralChannel | — | in-pr3 |
| `channels.ephemeral.duration-seconds_one` | {{count}} second | ephemeralChannel | — | in-pr3 |
| `channels.ephemeral.duration-seconds_other` | {{count}} seconds | ephemeralChannel | — | in-pr3 |
| `channels.ephemeral.remaining-days` | {{count}}d left | ephemeralChannel | — | in-pr3 |
| `channels.ephemeral.remaining-hours` | {{count}}h left | ephemeralChannel | — | in-pr3 |
| `channels.ephemeral.remaining-minutes` | {{count}}m left | ephemeralChannel | — | in-pr3 |
| `channels.ephemeral.remaining-now` | now | ephemeralChannel | — | in-pr3 |
| `channels.ephemeral.tooltip-cleanup-due` | Ephemeral channel. Cleanup is due now. | ephemeralChannel | — | in-pr3 |
| `channels.ephemeral.tooltip-no-ttl` | Ephemeral channel. Cleans up automatically after inactivity. | ephemeralChannel | — | in-pr3 |
| `channels.ephemeral.tooltip-remaining` | Ephemeral channel. Cleans up {{remaining}}. | ephemeralChannel | — | in-pr3 |
| `channels.ephemeral.tooltip-scheduled` | Ephemeral channel. Cleans up {{remaining}}. Scheduled for {{deadline}}. | ephemeralChannel | — | in-pr3 |
| `channels.ephemeral.tooltip-ttl` | Ephemeral channel. Cleans up after {{duration}} of inactivity. | ephemeralChannel | — | in-pr3 |
| `channels.ephemeral.ttl-days` | {{count}}d TTL | ephemeralChannel | — | in-pr3 |
| `channels.ephemeral.ttl-hours` | {{count}}h TTL | ephemeralChannel | — | in-pr3 |
| `channels.ephemeral.ttl-minutes` | {{count}}m TTL | ephemeralChannel | — | in-pr3 |
| `channels.ephemeral.ttl-seconds` | {{count}}s TTL | ephemeralChannel | — | in-pr3 |
| `channels.forum.posts-aria` | Forum posts | ForumChannelContent | — | in-pr3 |
| `channels.header.join` | Join | ChannelScreenHeader | — | in-pr3 |
| `channels.header.joining` | Joining… | ChannelScreenHeader | — | in-pr3 |
| `channels.header.open-profile` | Open profile for {{name}} | ChannelScreenHeader | — | in-pr3 |
| `channels.header.terminal-hide` | Hide Buzz Term | ChannelScreenHeader | — | in-pr3 |
| `channels.header.terminal-open` | Open Buzz Term | ChannelScreenHeader | — | in-pr3 |
| `channels.header.terminal-title` | Buzz Term (⌘J) | ChannelScreenHeader | — | in-pr3 |
| `channels.intro.add-agent` | Add agent | useChannelIntro | — | in-pr3 |
| `channels.intro.add-agent-description` | Add an agent here. | useChannelIntro | — | in-pr3 |
| `channels.intro.add-files` | Add files | useChannelIntro | — | in-pr3 |
| `channels.intro.add-files-description` | Add a repo. | useChannelIntro | — | in-pr3 |
| `channels.intro.add-people` | Add people | useChannelIntro | — | in-pr3 |
| `channels.intro.add-people-description` | Invite members. | useChannelIntro | — | in-pr3 |
| `channels.intro.browse-channels` | Browse channels | useChannelIntro | — | in-pr3 |
| `channels.intro.create-agent` | Create an agent | useChannelIntro | — | in-pr3 |
| `channels.intro.create-channel` | Create a channel | useChannelIntro | — | in-pr3 |
| `channels.intro.kind-welcome` | private welcome channel | useChannelIntro | — | in-pr3 |
| `channels.invite.add` | Add | AddMemberSearchResultRow, ChannelMemberInviteCard | — | in-pr3 |
| `channels.invite.add-members` | Add members | ChannelMemberInviteCard | — | in-pr3 |
| `channels.invite.adding` | Adding... | ChannelMemberInviteCard | — | in-pr3 |
| `channels.invite.agent-label` | agent | AddMemberSearchResultRow, ChannelMemberInviteCard | — | in-pr3 |
| `channels.invite.by-public-key` | by public key | ChannelMemberInviteCard | — | in-pr3 |
| `channels.invite.heading` | Add members | ChannelMemberInviteCard | — | in-pr3 |
| `channels.invite.managed-by` | managed by {{name}} | AddMemberSearchResultRow | — | in-pr3 |
| `channels.invite.no-matches` | No matching users. | ChannelMemberInviteCard | — | in-pr3 |
| `channels.invite.remove-person` | Remove {{name}} | ChannelMemberInviteCard | — | in-pr3 |
| `channels.invite.role-admin` | admin | ChannelMemberInviteCard | — | in-pr3 |
| `channels.invite.role-bot` | bot | ChannelMemberInviteCard | — | in-pr3 |
| `channels.invite.role-guest` | guest | ChannelMemberInviteCard | — | in-pr3 |
| `channels.invite.role-label` | Role | ChannelMemberInviteCard | — | in-pr3 |
| `channels.invite.role-member` | member | ChannelMemberInviteCard | — | in-pr3 |
| `channels.invite.search-people` | Search people | ChannelMemberInviteCard | — | in-pr3 |
| `channels.invite.search-placeholder` | Search people, or paste a public key | ChannelMemberInviteCard | — | in-pr3 |
| `channels.invite.searching` | Searching… | ChannelMemberInviteCard | — | in-pr3 |
| `channels.invite.select-candidate` | Select {{name}} | AddMemberSearchResultRow | — | in-pr3 |
| `channels.invite.selected-count` | {{total}} selected | ChannelMemberInviteCard | — | in-pr3 |
| `channels.lifecycle.ongoing` | Ongoing | channelLifecycle | — | in-pr3 |
| `channels.lifecycle.project` | Project | channelLifecycle | — | in-pr3 |
| `channels.lifecycle.temporary` | Temporary | channelLifecycle | — | in-pr3 |
| `channels.lifecycle.temporary-duration` | Temporary · {{duration}} | channelLifecycle | — | in-pr3 |
| `channels.manage.about-value` | About {{label}} | ChannelManagementSheetRows | — | in-pr3 |
| `channels.manage.add-description` | Add a description | ChannelManagementSheetRows | — | in-pr3 |
| `channels.manage.add-to-sidebar` | Add this channel to your sidebar | ChannelManagementSheet | — | in-pr3 |
| `channels.manage.archive-channel` | Archive channel | ChannelManagementSheet | — | in-pr3 |
| `channels.manage.archiving-channel` | Archiving channel... | ChannelManagementSheet | — | in-pr3 |
| `channels.manage.back-to-channel` | Back to channel | ChannelManagementSheet | — | in-pr3 |
| `channels.manage.cancel` | Cancel | ChannelManagementSheet | — | in-pr3 |
| `channels.manage.canvas` | Canvas | ChannelManagementSheet | — | in-pr3 |
| `channels.manage.canvas-help` | Use the canvas as a shared space for notes, plans, and other channel information. | ChannelManagementSheet | — | in-pr3 |
| `channels.manage.channel-id-label` | Channel ID | ChannelManagementSheet | — | in-pr3 |
| `channels.manage.channel-settings` | Channel Settings | ChannelManagementSheet | — | in-pr3 |
| `channels.manage.channel-settings-desc` | Channel settings | ChannelManagementSheet | — | in-pr3 |
| `channels.manage.copied-value` | Copied {{label}} | ChannelManagementSheetRows | — | in-pr3 |
| `channels.manage.copy-value` | Copy {{label}} | ChannelManagementSheetRows | — | in-pr3 |
| `channels.manage.delete-channel` | Delete channel | ChannelManagementSheet | — | in-pr3 |
| `channels.manage.description` | Description | ChannelManagementSheet | — | in-pr3 |
| `channels.manage.details` | Details | ChannelManagementSheet | — | in-pr3 |
| `channels.manage.edit-channel` | Edit channel | ChannelManagementSheetRows | — | in-pr3 |
| `channels.manage.edit-private-channel` | Edit private channel | ChannelManagementSheet | — | in-pr3 |
| `channels.manage.edit-public-channel` | Edit public channel | ChannelManagementSheet | — | in-pr3 |
| `channels.manage.edit-value` | Edit {{label}} | ChannelManagementSheetRows | — | in-pr3 |
| `channels.manage.join-channel` | Join channel | ChannelManagementSheet | — | in-pr3 |
| `channels.manage.joining-channel` | Joining channel... | ChannelManagementSheet | — | in-pr3 |
| `channels.manage.leave-channel` | Leave channel | ChannelManagementSheet | — | in-pr3 |
| `channels.manage.leaving-channel` | Leaving channel... | ChannelManagementSheet | — | in-pr3 |
| `channels.manage.loading` | Loading... | ChannelManagementSheet | — | in-pr3 |
| `channels.manage.member-count_one` | {{count}} member | ChannelManagementSheet | — | in-pr3 |
| `channels.manage.member-count_other` | {{count}} members | ChannelManagementSheet | — | in-pr3 |
| `channels.manage.members` | Members | ChannelManagementSheet | — | in-pr3 |
| `channels.manage.name` | Name | ChannelManagementSheet | — | in-pr3 |
| `channels.manage.open-value` | Open {{label}} | ChannelManagementSheetRows | — | in-pr3 |
| `channels.manage.private` | Private | ChannelManagementSheet | — | in-pr3 |
| `channels.manage.public` | Public | ChannelManagementSheet | — | in-pr3 |
| `channels.manage.restoring-channel` | Restoring channel... | ChannelManagementSheet | — | in-pr3 |
| `channels.manage.save-changes` | Save changes | ChannelManagementSheet | — | in-pr3 |
| `channels.manage.saving` | Saving... | ChannelManagementSheet | — | in-pr3 |
| `channels.manage.unarchive-channel` | Unarchive channel | ChannelManagementSheet | — | in-pr3 |
| `channels.manage.value-copied` | {{label}} copied | ChannelManagementSheetRows | — | in-pr3 |
| `channels.manage.visibility-label` | Visibility | ChannelManagementSheet | — | in-pr3 |
| `channels.manage.workflow-count_one` | {{count}} workflow | ChannelManagementSheet | — | in-pr3 |
| `channels.manage.workflow-count_other` | {{count}} workflows | ChannelManagementSheet | — | in-pr3 |
| `channels.manage.workflows` | Workflows | ChannelManagementSheet | — | in-pr3 |
| `channels.members.access-body` | Choose who can send instructions to {{name}}. | EditRespondToDialog | — | in-pr3 |
| `channels.members.add-agent-failed` | Failed to add agent. | MembersSidebar | — | in-pr3 |
| `channels.members.add-people-placeholder` | Add people and agents | MembersSidebar | — | in-pr3 |
| `channels.members.agent-label` | agent | MembersSidebarMemberCard | — | in-pr3 |
| `channels.members.archived` | Archived | MembersSidebar | — | in-pr3 |
| `channels.members.ban-from-community` | Ban from community | MembersSidebarMemberCard | — | in-pr3 |
| `channels.members.cancel` | Cancel | EditRespondToDialog | — | in-pr3 |
| `channels.members.change-role` | Change role | MembersSidebarMemberCard | — | in-pr3 |
| `channels.members.channel-actions` | Channel actions | ChannelMembersBar | — | in-pr3 |
| `channels.members.channel-settings` | Channel settings | ChannelMembersBar | — | in-pr3 |
| `channels.members.close` | Close | MembersSidebar | — | in-pr3 |
| `channels.members.control-failed` | Failed to control agent. | useMembersSidebarActions | — | in-pr3 |
| `channels.members.current-suffix` |  (current) | MembersSidebarMemberCard | — | in-pr3 |
| `channels.members.deployed` | Deployed {{name}}. | useMembersSidebarActions | — | in-pr3 |
| `channels.members.heading` | Members | ChannelMembersBar, MembersSidebar | — | in-pr3 |
| `channels.members.heading-count` | Members · {{total}} | MembersSidebar | — | in-pr3 |
| `channels.members.lift-ban` | Lift ban | MembersSidebarMemberCard | — | in-pr3 |
| `channels.members.lift-timeout` | Lift timeout | MembersSidebarMemberCard | — | in-pr3 |
| `channels.members.loading` | Loading members... | MembersSidebar | — | in-pr3 |
| `channels.members.manage-access` | Manage agent access... | MembersSidebarMemberCard | — | in-pr3 |
| `channels.members.manage-agent-access` | Manage agent access | EditRespondToDialog | — | in-pr3 |
| `channels.members.manage-channel` | Manage channel | ChannelMembersBar | — | in-pr3 |
| `channels.members.more-members` | {{total}} more members | ChannelMemberAvatarStack | — | in-pr3 |
| `channels.members.no-archived-results` | No archived members match your search. | MembersSidebar | — | in-pr3 |
| `channels.members.no-matches` | No matching people or agents. | MembersSidebar | — | in-pr3 |
| `channels.members.no-search-results` | No members match your search. | MembersSidebar | — | in-pr3 |
| `channels.members.none` | No members found. | MembersSidebar | — | in-pr3 |
| `channels.members.not-in-channel` | Not in this channel | MembersSidebar | — | in-pr3 |
| `channels.members.open-profile` | Open profile for {{name}} | MembersSidebarMemberCard | — | in-pr3 |
| `channels.members.remove-all-from-channel` | Remove all from channel | MembersSidebarAgentControls | — | in-pr3 |
| `channels.members.remove-bot-failed` | Failed to remove bot from channel. | useMembersSidebarActions | — | in-pr3 |
| `channels.members.remove-from-channel` | Remove from channel | MembersSidebarMemberCard | — | in-pr3 |
| `channels.members.remove-member-failed` | Failed to remove member. | useMembersSidebarActions | — | in-pr3 |
| `channels.members.removed-bots_one` | Removed {{count}} managed bot from this channel. | useMembersSidebarActions | — | in-pr3 |
| `channels.members.removed-bots_other` | Removed {{count}} managed bots from this channel. | useMembersSidebarActions | — | in-pr3 |
| `channels.members.respawn-failed` | Failed to respawn agent. | useMembersSidebarActions | — | in-pr3 |
| `channels.members.respawned` | Respawned {{name}}. | useMembersSidebarActions | — | in-pr3 |
| `channels.members.respond-anyone` | Anyone | MembersSidebarMemberCard | — | in-pr3 |
| `channels.members.respond-only-me` | Only me | MembersSidebarMemberCard | — | in-pr3 |
| `channels.members.respond-selected-people` | Selected people ({{total}}) | MembersSidebarMemberCard | — | in-pr3 |
| `channels.members.restarted-in-community` | Restarted {{name}} in this community. | useMembersSidebarActions | — | in-pr3 |
| `channels.members.role-admin` | admin | MembersSidebarMemberCard | — | in-pr3 |
| `channels.members.role-option-admin` | Admin | MembersSidebarMemberCard | — | in-pr3 |
| `channels.members.role-option-guest` | Guest | MembersSidebarMemberCard | — | in-pr3 |
| `channels.members.role-option-member` | Member | MembersSidebarMemberCard | — | in-pr3 |
| `channels.members.role-owner` | owner | MembersSidebarMemberCard | — | in-pr3 |
| `channels.members.save-access` | Save access | EditRespondToDialog | — | in-pr3 |
| `channels.members.saving` | Saving... | EditRespondToDialog | — | in-pr3 |
| `channels.members.search-people-placeholder` | Search people and agents | MembersSidebar | — | in-pr3 |
| `channels.members.searching` | Searching... | MembersSidebar | — | in-pr3 |
| `channels.members.shutdown-sent` | Shutdown command sent to {{name}}. | useMembersSidebarActions | — | in-pr3 |
| `channels.members.spawn-all` | Spawn or respawn all | MembersSidebarAgentControls | — | in-pr3 |
| `channels.members.spawned` | Spawned {{name}}. | useMembersSidebarActions | — | in-pr3 |
| `channels.members.spawned-agents_one` | Spawned or respawned {{count}} agent. | useMembersSidebarActions | — | in-pr3 |
| `channels.members.spawned-agents_other` | Spawned or respawned {{count}} agents. | useMembersSidebarActions | — | in-pr3 |
| `channels.members.started-in-community` | Started {{name}} in this community. | useMembersSidebarActions | — | in-pr3 |
| `channels.members.status-running` | Running | MembersSidebarMemberCard | — | in-pr3 |
| `channels.members.status-stopped` | Stopped | MembersSidebarMemberCard | — | in-pr3 |
| `channels.members.stop-all` | Stop all | MembersSidebarAgentControls | — | in-pr3 |
| `channels.members.stop-failed` | Failed to stop agent. | useMembersSidebarActions | — | in-pr3 |
| `channels.members.stopped-agent` | Stopped {{name}}. | useMembersSidebarActions | — | in-pr3 |
| `channels.members.stopped-agents_one` | Stopped or requested shutdown for {{count}} agent. | useMembersSidebarActions | — | in-pr3 |
| `channels.members.stopped-agents_other` | Stopped or requested shutdown for {{count}} agents. | useMembersSidebarActions | — | in-pr3 |
| `channels.members.stopped-in-community` | Stopped {{name}} in this community. | useMembersSidebarActions | — | in-pr3 |
| `channels.members.this-agent` | this agent | EditRespondToDialog | — | in-pr3 |
| `channels.members.time-out` | Time out | MembersSidebarMemberCard | — | in-pr3 |
| `channels.members.timeout-1-hour` | 1 hour | MembersSidebarMemberCard | — | in-pr3 |
| `channels.members.timeout-24-hours` | 24 hours | MembersSidebarMemberCard | — | in-pr3 |
| `channels.members.timeout-7-days` | 7 days | MembersSidebarMemberCard | — | in-pr3 |
| `channels.members.title` | Channel members | ChannelMembersBar, MembersSidebar | — | in-pr3 |
| `channels.members.view-activity` | View activity | MembersSidebarMemberCard | — | in-pr3 |
| `channels.members.view-members` | View channel members ({{total}}) | ChannelMembersBar | — | in-pr3 |
| `channels.moderation.archive-channel` | Archive channel | ChannelManagementModerationActions | — | in-pr3 |
| `channels.moderation.archiving-channel` | Archiving channel | ChannelManagementModerationActions | — | in-pr3 |
| `channels.moderation.cancel` | Cancel | ChannelManagementModerationActions | — | in-pr3 |
| `channels.moderation.delete-channel` | Delete channel | ChannelManagementModerationActions | — | in-pr3 |
| `channels.moderation.delete-confirm` | Delete channel? | ChannelManagementModerationActions | — | in-pr3 |
| `channels.moderation.delete-confirm-body` | Delete {{channelName}} from the community list. This action cannot be undone. | ChannelManagementModerationActions | — | in-pr3 |
| `channels.moderation.deleting` | Deleting... | ChannelManagementModerationActions | — | in-pr3 |
| `channels.moderation.restoring-channel` | Restoring channel | ChannelManagementModerationActions | — | in-pr3 |
| `channels.moderation.toast-ban-lifted` | Ban lifted | useMembersSidebarModeration | — | in-pr3 |
| `channels.moderation.toast-banned` | Member banned | useMembersSidebarModeration | — | in-pr3 |
| `channels.moderation.toast-failed` | Moderation action failed | useMembersSidebarModeration | — | in-pr3 |
| `channels.moderation.toast-timed-out` | Member timed out | useMembersSidebarModeration | — | in-pr3 |
| `channels.moderation.toast-timeout-lifted` | Timeout lifted | useMembersSidebarModeration | — | in-pr3 |
| `channels.moderation.unarchive-channel` | Unarchive channel | ChannelManagementModerationActions | — | in-pr3 |
| `channels.pane.empty-description` | Messages and sub-replies will appear here once the relay has history for this channel. | ChannelPane | — | in-pr3 |
| `channels.pane.empty-forum-description` | Select a stream or DM to load real message history in this first integration pass. | ChannelPane | — | in-pr3 |
| `channels.pane.empty-forum-title` | Forum channels are next | ChannelPane | — | in-pr3 |
| `channels.pane.empty-no-channel` | No channel selected | ChannelPane | — | in-pr3 |
| `channels.pane.empty-title` | No messages yet | ChannelPane | — | in-pr3 |
| `channels.pane.finish-edit-first` | Finish or cancel your edit first. | useRoutedMessageEdit | — | in-pr3 |
| `channels.pane.finish-edit-thread` | Finish or cancel your edit before leaving the thread. | useChannelPaneHandlers | — | in-pr3 |
| `channels.pane.join-participate` | Join to participate | ChannelPane | — | in-pr3 |
| `channels.pane.joining` | Joining... | ChannelPane | — | in-pr3 |
| `channels.pane.messages-aria` | Channel messages and composer | ChannelPane | — | in-pr3 |
| `channels.pane.panel` | Panel | ChannelPane | — | in-pr3 |
| `channels.pane.placeholder-archived` | Archived channels are read-only. | ChannelPane | — | in-pr3 |
| `channels.pane.placeholder-channel` | Message #{{channelName}} | ChannelPane | — | in-pr3 |
| `channels.pane.placeholder-dm` | Message {{displayName}} | ChannelPane | — | in-pr3 |
| `channels.pane.placeholder-forum` | Forum posting is not wired in this pass. | ChannelPane | — | in-pr3 |
| `channels.pane.placeholder-none` | Select a channel | ChannelPane | — | in-pr3 |
| `channels.pane.placeholder-readonly` | This channel is read-only. | ChannelPane | — | in-pr3 |
| `channels.pane.placeholder-timeout` | You're timed out by community moderators. | ChannelPane | — | in-pr3 |
| `channels.pane.viewing` | Viewing | ChannelPane | — | in-pr3 |
| `channels.permissions.private` | Private | ChannelPermissionsSettings | — | in-pr3 |
| `channels.permissions.public` | Public | ChannelPermissionsSettings | — | in-pr3 |
| `channels.permissions.visibility-label` | Visibility | ChannelPermissionsSettings | — | in-pr3 |
| `channels.permissions.visibility-value` | Visibility: {{label}} | ChannelPermissionsSettings | — | in-pr3 |
| `channels.thread.back-to` | Back to #{{channelName}} | FocusThreadDrawer | — | in-pr3 |
| `channels.thread.expand` | Expand thread | ThreadViewModeToggle | — | in-pr3 |
| `channels.thread.label` | Thread | FocusThreadDrawer | — | in-pr3 |
| `channels.thread.resize-drag` | Drag to resize. | RightAuxiliaryPane | — | in-pr3 |
| `channels.thread.resize-drag-reset` | Drag to resize. Double-click to reset width. | RightAuxiliaryPane | — | in-pr3 |
| `channels.thread.resize-panel` | Resize panel | RightAuxiliaryPane | — | in-pr3 |
| `channels.thread.show-beside` | Show thread beside channel | ThreadViewModeToggle | — | in-pr3 |
| `channels.type.channel-type` | Channel type | ChannelTypeSettings | — | in-pr3 |
| `channels.type.channel-type-value` | Channel type: {{label}} | ChannelTypePicker | — | in-pr3 |
| `channels.type.current-duration` | Current ({{duration}}) | ChannelTypeSettings | — | in-pr3 |
| `channels.type.expires-after` | Expires after | ChannelTypeSettings | — | in-pr3 |
| `channels.type.ongoing` | Ongoing | ChannelTypePicker, ChannelTypeSettings | — | in-pr3 |
| `channels.type.ongoing-channel` | Ongoing channel | ChannelTypePicker | — | in-pr3 |
| `channels.type.project` | Project | ChannelTypePicker | — | in-pr3 |
| `channels.type.project-channel` | Project channel | ChannelTypePicker | — | in-pr3 |
| `channels.type.temporary` | Temporary | ChannelTypePicker, ChannelTypeSettings | — | in-pr3 |
| `channels.type.temporary-channel` | Temporary channel | ChannelTypePicker | — | in-pr3 |
| `channels.type.ttl-1-day` | 1 day | ChannelTypeSettings | — | in-pr3 |
| `channels.type.ttl-1-hour` | 1 hour | ChannelTypeSettings | — | in-pr3 |
| `channels.type.ttl-12-hours` | 12 hours | ChannelTypeSettings | — | in-pr3 |
| `channels.type.ttl-14-days` | 14 days | ChannelTypeSettings | — | in-pr3 |
| `channels.type.ttl-3-days` | 3 days | ChannelTypeSettings | — | in-pr3 |
| `channels.type.ttl-30-days` | 30 days | ChannelTypeSettings | — | in-pr3 |
| `channels.type.ttl-30-minutes` | 30 minutes | ChannelTypeSettings | — | in-pr3 |
| `channels.type.ttl-6-hours` | 6 hours | ChannelTypeSettings | — | in-pr3 |
| `channels.type.ttl-7-days` | 7 days | ChannelTypeSettings | — | in-pr3 |
| `channels.welcome.banner-complete` | Nice work. | WelcomeComposerBanner | — | in-pr3 |
| `channels.welcome.banner-dismiss` | Dismiss hint | WelcomeComposerBanner | — | in-pr3 |
| `channels.welcome.banner-mention-prefix` | Mention  | WelcomeComposerBanner | — | in-pr3 |
| `channels.welcome.banner-mention-suffix` |  or another teammate whenever you want their help. | WelcomeComposerBanner | — | in-pr3 |
| `channels.welcome.banner-setting-up` | Setting up your welcome team… | WelcomeComposerBanner | — | in-pr3 |
| `channels.welcome.cancel` | Cancel | WelcomeAgentCreateDialog | — | in-pr3 |
| `channels.welcome.create-manually` | Create manually | WelcomeAgentCreateDialog | — | in-pr3 |
| `channels.welcome.create-manually-body` | Fill in the agent’s name, instructions, and settings yourself. | WelcomeAgentCreateDialog | — | in-pr3 |
| `channels.welcome.create-with-guide` | Create with {{guideName}} | WelcomeAgentCreateDialog | — | in-pr3 |
| `channels.welcome.create-with-guide-body` | Talk through what you need. {{guideName}} will prepare a draft you can review and edit. | WelcomeAgentCreateDialog | — | in-pr3 |
| `channels.welcome.dialog-body` | Start with a conversation, or set everything up yourself. | WelcomeAgentCreateDialog | — | in-pr3 |
| `channels.welcome.dialog-title` | Create an agent | WelcomeAgentCreateDialog | — | in-pr3 |
| `channels.welcome.error-guide-unavailable` | The welcome guide is unavailable. Create the agent manually instead. | useWelcomeAgentCreate | — | in-pr3 |
| `channels.welcome.error-send-failed` | Could not start the conversation. | useWelcomeAgentCreate | — | in-pr3 |
| `channels.workflows.empty` | No workflows in this channel yet. | ChannelWorkflowsSection | — | in-pr3 |
| `channels.workflows.loading` | Loading workflows... | ChannelWorkflowsSection | — | in-pr3 |
| `channels.workflows.new-workflow` | New workflow | ChannelWorkflowsSection | — | in-pr3 |
| `channels.workflows.retry` | Retry | ChannelWorkflowsSection | — | in-pr3 |

| `messages.action.copy-link` | Copy link | MessageActionBar | — | in-pr3 |
| `messages.action.copy-message` | Copy message | MessageActionBar | — | in-pr3 |
| `messages.action.delete-message` | Delete message | MessageActionBar | — | in-pr3 |
| `messages.action.edit-message` | Edit message | MessageActionBar | — | in-pr3 |
| `messages.action.follow-thread` | Follow thread | MessageActionBar | — | in-pr3 |
| `messages.action.link-copied` | Link copied to clipboard | MessageActionBar | — | in-pr3 |
| `messages.action.mark-read` | Mark read | MessageActionBar | — | in-pr3 |
| `messages.action.mark-unread` | Mark unread | MessageActionBar | — | in-pr3 |
| `messages.action.message-copied` | Message copied to clipboard | MessageActionBar | — | in-pr3 |
| `messages.action.more-actions` | More actions | MessageActionBar | — | in-pr3 |
| `messages.action.react-with` | React with {{name}} | MessageActionBar | — | in-pr3 |
| `messages.action.remind-later` | Remind me later | MessageActionBar | — | in-pr3 |
| `messages.action.reply` | Reply | MessageActionBar | — | in-pr3 |
| `messages.action.report-message` | Report message | MessageActionBar | — | in-pr3 |
| `messages.action.send-to-channel` | Send to channel | MessageActionBar | — | in-pr3 |
| `messages.action.send-to-channel-failed` | Couldn't send to channel | MessageActionBar | — | in-pr3 |
| `messages.action.sent-to-channel` | Sent to channel | MessageActionBar | — | in-pr3 |
| `messages.action.unfollow-thread` | Unfollow thread | MessageActionBar | — | in-pr3 |
| `messages.agent.could-not-start` | Could not start {{agentName}} — your message was sent, but the agent may not respond. {{detail}} | useDetachedAgentStart | — | in-pr3 |
| `messages.agent.start-failed` | Could not start agent. | useDetachedAgentStart | — | in-pr3 |
| `messages.agent.still-connecting` | Buzz is still connecting to this community — mention the agent again in a moment. | useDetachedAgentStart | — | in-pr3 |
| `messages.agent.switched-before-start` | You switched community or identity before it could start. | useDetachedAgentStart | — | in-pr3 |
| `messages.attachment.cancel-upload` | Cancel upload | ComposerAttachments | — | in-pr3 |
| `messages.attachment.close` | Close | ComposerAttachments | — | in-pr3 |
| `messages.attachment.close-lightbox` | Close lightbox | ComposerAttachments | — | in-pr3 |
| `messages.attachment.draw-on-image` | Draw on image | ComposerAttachments | — | in-pr3 |
| `messages.attachment.drop-files` | Drop files to upload | ComposerAttachments | — | in-pr3 |
| `messages.attachment.file-fallback` | file {{hash}} | ComposerAttachments | — | in-pr3 |
| `messages.attachment.mark-spoiler` | Mark as spoiler | ComposerAttachments | — | in-pr3 |
| `messages.attachment.media-label` | Attachment {{hash}} | ComposerAttachments | — | in-pr3 |
| `messages.attachment.preview-description` | Full-size attachment preview. Press Escape or click outside to close. | ComposerAttachments | — | in-pr3 |
| `messages.attachment.preview-title` | {{name}} preview | ComposerAttachments | — | in-pr3 |
| `messages.attachment.queued-file` | Attachment | ComposerAttachments | — | in-pr3 |
| `messages.attachment.queued-thumbnail` | Queued attachment | ComposerAttachments | — | in-pr3 |
| `messages.attachment.remove` | Remove | ComposerAttachments | — | in-pr3 |
| `messages.attachment.remove-attachment` | Remove attachment | ComposerAttachments | — | in-pr3 |
| `messages.attachment.remove-name` | Remove {{name}} | ComposerAttachments | — | in-pr3 |
| `messages.attachment.remove-spoiler` | Remove spoiler | ComposerAttachments | — | in-pr3 |
| `messages.attachment.remove-voice-note` | Remove voice note | ComposerAttachments | — | in-pr3 |
| `messages.attachment.revert` | Revert | ComposerAttachments | — | in-pr3 |
| `messages.attachment.revert-tooltip` | Revert to original | ComposerAttachments | — | in-pr3 |
| `messages.attachment.size-bytes` | {{size}} B | ComposerAttachments | — | in-pr3 |
| `messages.attachment.size-kilobytes` | {{size}} KB | ComposerAttachments | — | in-pr3 |
| `messages.attachment.size-megabytes` | {{size}} MB | ComposerAttachments | — | in-pr3 |
| `messages.attachment.snapshot-agent` | Agent | ComposerAttachments | — | in-pr3 |
| `messages.attachment.snapshot-team` | Team | ComposerAttachments | — | in-pr3 |
| `messages.attachment.uploading-attachment` | Uploading attachment | ComposerAttachments | — | in-pr3 |
| `messages.attachment.uploading-name` | Uploading {{name}} | ComposerAttachments | — | in-pr3 |
| `messages.attachment.uploading-video` | Uploading video | ComposerAttachments | — | in-pr3 |
| `messages.attachment.video-label` | Video attachment {{hash}} | ComposerAttachments | — | in-pr3 |
| `messages.attachment.voice-note` | Voice note | ComposerAttachments | — | in-pr3 |
| `messages.audio.download` | Download | AudioMessageAttachment | — | in-pr3 |
| `messages.audio.download-failed` | Download failed | AudioMessageAttachment | — | in-pr3 |
| `messages.audio.download-name` | Download {{name}} | AudioMessageAttachment | — | in-pr3 |
| `messages.audio.loading` | Loading voice note | AudioMessageAttachment | — | in-pr3 |
| `messages.audio.pause` | Pause voice note | AudioMessageAttachment | — | in-pr3 |
| `messages.audio.play` | Play voice note | AudioMessageAttachment | — | in-pr3 |
| `messages.audio.playback-position` | Voice note playback position | AudioMessageAttachment | — | in-pr3 |
| `messages.audio.playback-speed` | Playback speed {{current}}; next {{next}} | AudioMessageAttachment | — | in-pr3 |
| `messages.audio.remove` | Remove | AudioMessageAttachment | — | in-pr3 |
| `messages.audio.remove-voice-note` | Remove voice note | AudioMessageAttachment | — | in-pr3 |
| `messages.audio.retry` | Retry voice note | AudioMessageAttachment | — | in-pr3 |
| `messages.audio.unavailable` | Audio unavailable. Retry playback. | AudioMessageAttachment | — | in-pr3 |
| `messages.audio.waveform-unavailable` | Waveform preview unavailable. Playback may still work. | AudioMessageAttachment | — | in-pr3 |
| `messages.composer.attach-file` | Attach file | MessageComposerToolbar | — | in-pr3 |
| `messages.composer.cancel-edit` | Cancel edit | ComposerReplyEditBanner | — | in-pr3 |
| `messages.composer.cancel-reply` | Cancel reply | ComposerReplyEditBanner | — | in-pr3 |
| `messages.composer.close-formatting` | Close formatting | MessageComposerToolbar | — | in-pr3 |
| `messages.composer.dismiss` | Dismiss | MessageComposer | — | in-pr3 |
| `messages.composer.edit-placeholder` | Edit your message | MessageComposer | — | in-pr3 |
| `messages.composer.editing-message` | Editing message | ComposerReplyEditBanner | — | in-pr3 |
| `messages.composer.finish-voice-note` | Finish voice note | ComposerAddressControls | — | in-pr3 |
| `messages.composer.formatting` | Formatting | MessageComposerToolbar | — | in-pr3 |
| `messages.composer.insert-emoji` | Insert emoji | ComposerEmojiPicker | — | in-pr3 |
| `messages.composer.insert-emoji-or-gif` | Insert emoji or GIF | ComposerEmojiPicker | — | in-pr3 |
| `messages.composer.manage-mentions` | Manage mentions | ComposerAddressControls | — | in-pr3 |
| `messages.composer.mention-someone` | Mention someone | ComposerAddressControls | — | in-pr3 |
| `messages.composer.more-addressed-agents_one` | {{count}} more addressed agent | ComposerAddressControls | — | in-pr3 |
| `messages.composer.more-addressed-agents_other` | {{count}} more addressed agents | ComposerAddressControls | — | in-pr3 |
| `messages.composer.record-voice-note` | Record voice note | MessageComposerToolbar | — | in-pr3 |
| `messages.composer.reply-placeholder` | Reply to {{author}} in #{{channelName}} | MessageComposer | — | in-pr3 |
| `messages.composer.replying-to` | Replying to {{author}} | ComposerReplyEditBanner | — | in-pr3 |
| `messages.composer.send-message` | Send message | ComposerAddressControls | — | in-pr3 |
| `messages.composer.sending` | Sending | ComposerAddressControls | — | in-pr3 |
| `messages.composer.tab-emoji` | Emoji | ComposerEmojiPicker | — | in-pr3 |
| `messages.composer.tab-gifs` | GIFs | ComposerEmojiPicker | — | in-pr3 |
| `messages.composer.toggle-formatting` | Toggle formatting | MessageComposerToolbar | — | in-pr3 |
| `messages.composer.tooltip-emoji` | Emoji | ComposerEmojiPicker | — | in-pr3 |
| `messages.composer.tooltip-emoji-and-gifs` | Emoji and GIFs | ComposerEmojiPicker | — | in-pr3 |
| `messages.composer.turn-off` | Turn off | ComposerAddressControls | — | in-pr3 |
| `messages.composer.unaddress-agent` | Don't automatically mention {{name}} in this thread | ComposerAddressControls | — | in-pr3 |
| `messages.composer.upload-failed` | Upload failed: {{message}} | MessageComposer | — | in-pr3 |
| `messages.delete.body` | This will permanently delete this message and cannot be undone. | DeleteMessageConfirmDialog | — | in-pr3 |
| `messages.delete.cancel` | Cancel | DeleteMessageConfirmDialog | — | in-pr3 |
| `messages.delete.confirm` | Delete | DeleteMessageConfirmDialog | — | in-pr3 |
| `messages.delete.title` | Delete message? | DeleteMessageConfirmDialog | — | in-pr3 |
| `messages.diff.expand` | Expand diff | DiffMessage | — | in-pr3 |
| `messages.diff.truncated` | Diff truncated. | DiffMessage | — | in-pr3 |
| `messages.diff.type-copied` | Copied | parseDiff | — | in-pr3 |
| `messages.diff.type-deleted` | Deleted | parseDiff | — | in-pr3 |
| `messages.diff.type-modified` | Modified | parseDiff | — | in-pr3 |
| `messages.diff.type-new-file` | New file | parseDiff | — | in-pr3 |
| `messages.diff.type-renamed` | Renamed | parseDiff | — | in-pr3 |
| `messages.diff.view-full-on` | View full diff on {{host}} | DiffMessage | — | in-pr3 |
| `messages.diff.view-full-source` | View the full diff at the source repository. | DiffMessage | — | in-pr3 |
| `messages.diff.view-split` | Split | DiffMessageExpanded | — | in-pr3 |
| `messages.diff.view-unified` | Unified | DiffMessageExpanded | — | in-pr3 |
| `messages.diff.viewer-title` | Diff Viewer | DiffMessageExpanded | — | in-pr3 |
| `messages.drafts.attachment-count_one` | {{count}} attachment | DraftDetailPane, DraftsPanel | — | in-pr3 |
| `messages.drafts.attachment-count_other` | {{count}} attachments | DraftDetailPane, DraftsPanel | — | in-pr3 |
| `messages.drafts.back-to-list` | Back to drafts list | DraftDetailPane | — | in-pr3 |
| `messages.drafts.cancel` | Cancel | DraftsPanel | — | in-pr3 |
| `messages.drafts.delete` | Delete | DraftDetailPane | — | in-pr3 |
| `messages.drafts.delete-draft` | Delete draft | DraftsPanel | — | in-pr3 |
| `messages.drafts.empty-draft` | Empty draft | DraftsPanel | — | in-pr3 |
| `messages.drafts.heading` | Drafts | DraftsPanel | — | in-pr3 |
| `messages.drafts.label` | Draft | DraftDetailPane | — | in-pr3 |
| `messages.drafts.no-channel-link` | No channel link | DraftsPanel | — | in-pr3 |
| `messages.drafts.none` | No drafts | DraftsPanel | — | in-pr3 |
| `messages.drafts.open-draft` | Open draft | DraftDetailPane, DraftsPanel | — | in-pr3 |
| `messages.drafts.orphaned-notice` | The original thread was deleted. This draft can no longer be opened or sent. | DraftDetailPane | — | in-pr3 |
| `messages.drafts.select-heading` | Select a draft | DraftDetailPane | — | in-pr3 |
| `messages.drafts.select-hint` | Pick a draft to preview it and choose what to do next. | DraftDetailPane | — | in-pr3 |
| `messages.drafts.send` | Send | DraftDetailPane, DraftsPanel | — | in-pr3 |
| `messages.drafts.send-confirm-body` | Are you sure you want to send this message to {{destination}}? | DraftsPanel | — | in-pr3 |
| `messages.drafts.send-message` | Send message | DraftsPanel | — | in-pr3 |
| `messages.drafts.thread-deleted` | Thread deleted | DraftsPanel | — | in-pr3 |
| `messages.drafts.thread-deleted-badge` | thread deleted | DraftsPanel | — | in-pr3 |
| `messages.drafts.unknown-channel` | Unknown channel | DraftDetailPane, DraftsPanel | — | in-pr3 |
| `messages.drafts.unknown-time` | Unknown time | DraftsPanel | — | in-pr3 |
| `messages.drafts.view-draft-in` | View draft in {{channel}} | DraftsPanel | — | in-pr3 |
| `messages.drafts.you` | You | DraftDetailPane | — | in-pr3 |
| `messages.editor.placeholder` | Write a message… | useRichTextEditor | — | in-pr3 |
| `messages.format.bold` | Bold | FormattingToolbar | — | in-pr3 |
| `messages.format.bullet-list` | Bullet list | FormattingToolbar | — | in-pr3 |
| `messages.format.code` | Code | FormattingToolbar | — | in-pr3 |
| `messages.format.code-block` | Code block | FormattingToolbar | — | in-pr3 |
| `messages.format.enter-url` | Enter URL: | FormattingToolbar | — | in-pr3 |
| `messages.format.italic` | Italic | FormattingToolbar | — | in-pr3 |
| `messages.format.link` | Link | FormattingToolbar | — | in-pr3 |
| `messages.format.link-text` | Link text: | FormattingToolbar | — | in-pr3 |
| `messages.format.ordered-list` | Ordered list | FormattingToolbar | — | in-pr3 |
| `messages.format.quote` | Quote | FormattingToolbar | — | in-pr3 |
| `messages.format.selection-formatting` | Selection formatting | SelectionFormattingTray | — | in-pr3 |
| `messages.format.spoiler` | Spoiler | FormattingToolbar | — | in-pr3 |
| `messages.format.strikethrough` | Strikethrough | FormattingToolbar | — | in-pr3 |
| `messages.format.tooltip-with-shortcut` | {{label}} ({{shortcut}}) | FormattingToolbar | — | in-pr3 |
| `messages.image-editor.cancel` | Cancel | ComposerImageEditor | — | in-pr3 |
| `messages.image-editor.canvas-aria` | Drawing canvas | ComposerImageEditor | — | in-pr3 |
| `messages.image-editor.color-black` | Black | ComposerImageEditor | — | in-pr3 |
| `messages.image-editor.color-blue` | Blue | ComposerImageEditor | — | in-pr3 |
| `messages.image-editor.color-green` | Green | ComposerImageEditor | — | in-pr3 |
| `messages.image-editor.color-red` | Red | ComposerImageEditor | — | in-pr3 |
| `messages.image-editor.color-white` | White | ComposerImageEditor | — | in-pr3 |
| `messages.image-editor.color-yellow` | Yellow | ComposerImageEditor | — | in-pr3 |
| `messages.image-editor.pen-color-aria` | {{color}} pen | ComposerImageEditor | — | in-pr3 |
| `messages.image-editor.redo-aria` | Redo stroke | ComposerImageEditor | — | in-pr3 |
| `messages.image-editor.redo-tooltip` | Redo (⇧⌘Z) | ComposerImageEditor | — | in-pr3 |
| `messages.image-editor.save` | Save | ComposerImageEditor | — | in-pr3 |
| `messages.image-editor.save-failed` | Could not save the drawing. Please try again. | ComposerImageEditor | — | in-pr3 |
| `messages.image-editor.stroke-width` | Stroke width | ComposerImageEditor | — | in-pr3 |
| `messages.image-editor.undo-aria` | Undo last stroke | ComposerImageEditor | — | in-pr3 |
| `messages.image-editor.undo-tooltip` | Undo (⌘Z) | ComposerImageEditor | — | in-pr3 |
| `messages.link.add` | Add link | useLinkEditor | — | in-pr3 |
| `messages.link.buzz-link` | Buzz link | composerMessageLinkNode | — | in-pr3 |
| `messages.link.cancel` | Cancel | useLinkEditor | — | in-pr3 |
| `messages.link.display-text` | Display text | useLinkEditor | — | in-pr3 |
| `messages.link.display-text-placeholder` | Text to display | useLinkEditor | — | in-pr3 |
| `messages.link.edit` | Edit link | useLinkEditor | — | in-pr3 |
| `messages.link.fallback-channel-name` | channel | composerMessageLinkNode | — | in-pr3 |
| `messages.link.loading-details` | Loading link preview details | useComposerLinkPreviews | — | in-pr3 |
| `messages.link.loading-preview` | Loading link preview | useComposerLinkPreviews | — | in-pr3 |
| `messages.link.open-channel` | Open channel {{name}} | composerMessageLinkNode | — | in-pr3 |
| `messages.link.open-issue` | Open issue {{id}} in repository {{name}} | composerMessageLinkNode | — | in-pr3 |
| `messages.link.open-name` | Open {{name}} | useComposerLinkPreviews | — | in-pr3 |
| `messages.link.open-project` | Open project {{name}} | composerMessageLinkNode | — | in-pr3 |
| `messages.link.open-pull-request` | Open pull request {{id}} in repository {{name}} | composerMessageLinkNode | — | in-pr3 |
| `messages.link.open-repository` | Open repository {{name}} | composerMessageLinkNode | — | in-pr3 |
| `messages.link.remove` | Remove | useLinkEditor | — | in-pr3 |
| `messages.link.save` | Save | useLinkEditor | — | in-pr3 |
| `messages.link.send-without-previews` | Send without link previews | useComposerLinkPreviews | — | in-pr3 |
| `messages.link.unlink` | Unlink | useLinkEditor | — | in-pr3 |
| `messages.link.url` | URL | useLinkEditor | — | in-pr3 |
| `messages.mention.agent-label` | agent | MentionAutocomplete | — | in-pr3 |
| `messages.mention.agent-will-auto-mention` | Agent will be mentioned automatically | useAutoPinMentionedAgents | — | in-pr3 |
| `messages.mention.agents-will-auto-mention` | {{count}} agents will be mentioned automatically | useAutoPinMentionedAgents | — | in-pr3 |
| `messages.mention.always-address-aria` | Automatically mention {{name}} | MentionAutocomplete | — | in-pr3 |
| `messages.mention.auto-mention-agents` | Automatically mention agents | MentionAutocomplete | — | in-pr3 |
| `messages.mention.auto-mention-hint` | Address selected agents in thread replies | MentionAutocomplete | — | in-pr3 |
| `messages.mention.cancel` | Cancel | NonMemberMentionDialog | — | in-pr3 |
| `messages.mention.denied-still-send` | You can still send without inviting them. | NonMemberMentionDialog | — | in-pr3 |
| `messages.mention.dialog-title` | Mention people outside this channel? | NonMemberMentionDialog | — | in-pr3 |
| `messages.mention.do-nothing` | Do nothing | NonMemberMentionDialog | — | in-pr3 |
| `messages.mention.invite` | Invite | NonMemberMentionDialog | — | in-pr3 |
| `messages.mention.invite-failed` | Could not invite members. | useNonMemberInvite | — | in-pr3 |
| `messages.mention.invite-or-cancel` | Invite them to the channel, or cancel to keep your draft. | NonMemberMentionDialog | — | in-pr3 |
| `messages.mention.invite-or-send` | Invite them to the channel, or send without inviting them. | NonMemberMentionDialog | — | in-pr3 |
| `messages.mention.inviting` | Inviting... | NonMemberMentionDialog | — | in-pr3 |
| `messages.mention.managed-by` | managed by {{owner}} | MentionAutocomplete | — | in-pr3 |
| `messages.mention.managed-by-outside` | managed by {{owner}} · not in channel | MentionAutocomplete | — | in-pr3 |
| `messages.mention.mention-aria` | Mention {{name}} | MentionAutocomplete | — | in-pr3 |
| `messages.mention.never-address-aria` | Don't automatically mention {{name}} in this thread | MentionAutocomplete | — | in-pr3 |
| `messages.mention.nonmember-names_one` | {{names}} is not in this channel. | NonMemberMentionDialog | — | in-pr3 |
| `messages.mention.nonmember-names_other` | {{names}} are not in this channel. | NonMemberMentionDialog | — | in-pr3 |
| `messages.mention.not-in-channel` | not in channel | MentionAutocomplete | — | in-pr3 |
| `messages.mention.send-anyway` | Send anyway | NonMemberMentionDialog | — | in-pr3 |
| `messages.mention.team-agent-count_one` | team · {{count}} agent | MentionAutocomplete | — | in-pr3 |
| `messages.mention.team-agent-count_other` | team · {{count}} agents | MentionAutocomplete | — | in-pr3 |
| `messages.mention.tooltip-always-address` | Automatically mention | MentionAutocomplete | — | in-pr3 |
| `messages.mention.tooltip-never-address` | Don't automatically mention in this thread | MentionAutocomplete | — | in-pr3 |
| `messages.mention.will-auto-mention` | {{name}} will be mentioned automatically | useAutoPinMentionedAgents | — | in-pr3 |
| `messages.new-message.add-aria` | Add {{name}} | NewMessageResultRow | — | in-pr3 |
| `messages.new-message.agent-label` | agent | NewMessageResultRow | — | in-pr3 |
| `messages.new-message.already-added-aria` | Already added {{name}} | NewMessageResultRow | — | in-pr3 |
| `messages.new-message.choose-recipient-first` | Choose at least one recipient first. | NewMessageScreen | — | in-pr3 |
| `messages.new-message.loading-results` | Loading people and agents | NewMessageScreen | — | in-pr3 |
| `messages.new-message.managed-by` | managed by {{owner}} | NewMessageResultRow | — | in-pr3 |
| `messages.new-message.no-matches` | No matching users. | NewMessageScreen | — | in-pr3 |
| `messages.new-message.no-people` | No people or agents available to message. | NewMessageScreen | — | in-pr3 |
| `messages.new-message.open-dm-failed` | Failed to open direct message. | NewMessageScreen | — | in-pr3 |
| `messages.new-message.opening` | Opening… | NewMessageScreen | — | in-pr3 |
| `messages.new-message.placeholder-direct` | Message {{name}} | NewMessageScreen | — | in-pr3 |
| `messages.new-message.placeholder-group` | Message {{count}} people | NewMessageScreen | — | in-pr3 |
| `messages.new-message.placeholder-no-recipient` | Choose a recipient to start a message | NewMessageScreen | — | in-pr3 |
| `messages.new-message.recipient-limit` | DMs support up to nine people, including you. | NewMessageScreen | — | in-pr3 |
| `messages.new-message.send-failed` | Failed to send message. | NewMessageScreen | — | in-pr3 |
| `messages.new-message.to-aria` | To | NewMessageScreen | — | in-pr3 |
| `messages.new-message.to-field` | To: | NewMessageScreen | — | in-pr3 |
| `messages.reaction.add-aria` | Add reaction | MessageReactions | — | in-pr3 |
| `messages.reaction.failed` | Failed to update the reaction. | useReactionHandler | — | in-pr3 |
| `messages.reaction.many-reactors` | {{names}}, and {{last}} | MessageReactions | — | in-pr3 |
| `messages.reaction.name-separator` | ,  | MessageReactions | — | in-pr3 |
| `messages.reaction.open-aria` | Open reactions | MessageActionBar, SystemMessageRow | — | in-pr3 |
| `messages.reaction.people-count_one` | {{count}} person | MessageReactions | — | in-pr3 |
| `messages.reaction.people-count_other` | {{count}} people | MessageReactions | — | in-pr3 |
| `messages.reaction.react` | React | MessageActionBar, MessageReactions, SystemMessageRow | — | in-pr3 |
| `messages.reaction.reacted-with` | reacted with | MessageReactions | — | in-pr3 |
| `messages.reaction.toggle-aria` | Toggle {{emoji}} reaction | MessageReactions | — | in-pr3 |
| `messages.reaction.two-reactors` | {{first}} and {{second}} | MessageReactions | — | in-pr3 |
| `messages.reaction.you-click-to-remove` | You (click to remove) | MessageReactions | — | in-pr3 |
| `messages.row.agent-managed-by` | Agent managed by | MessageAgentOwner | — | in-pr3 |
| `messages.row.anyone-can-instruct` | Anyone can send instructions to this agent | MessageRow | — | in-pr3 |
| `messages.row.collapse-descendants-aria` | Collapse replies to this message | MessageRow | — | in-pr3 |
| `messages.row.edited` | (edited) | MessageRow | — | in-pr3 |
| `messages.row.edited-tooltip` | This message has been edited | MessageRow | — | in-pr3 |
| `messages.row.loading-diff` | Loading diff… | MessageRow | — | in-pr3 |
| `messages.row.loading-diff-viewer` | Loading diff viewer… | MessageRow | — | in-pr3 |
| `messages.row.managed-by` | managed by | MessageAgentOwner | — | in-pr3 |
| `messages.row.owner-unavailable` | owner unavailable | MessageAgentOwner | — | in-pr3 |
| `messages.row.owner-unavailable-aria` | Agent; owner unavailable | MessageAgentOwner | — | in-pr3 |
| `messages.row.remove-previews-failed` | Failed to remove previews: {{error}} | MessageRow | — | in-pr3 |
| `messages.row.selected-can-instruct` | Selected people can send instructions to this agent | MessageRow | — | in-pr3 |
| `messages.row.sending` | Sending… | MessageRow | — | in-pr3 |
| `messages.row.sent-from-thread` | Sent from thread: | SentFromThreadLine | — | in-pr3 |
| `messages.row.unread-divider-aria` | New messages | UnreadDivider | — | in-pr3 |
| `messages.row.unread-divider-label` | New | UnreadDivider | — | in-pr3 |
| `messages.system.added-by` | added by | systemEventCopy | — | in-pr3 |
| `messages.system.along-with` |  along with | SystemMessageRow | — | in-pr3 |
| `messages.system.along-with-comma` | , along with | SystemMessageRow | — | in-pr3 |
| `messages.system.archived-channel` | archived this channel | SystemMessageRow | — | in-pr3 |
| `messages.system.arrived` | arrived | SystemMessageRow | — | in-pr3 |
| `messages.system.arrived-along-with` | arrived along with | SystemMessageRow | — | in-pr3 |
| `messages.system.changed-purpose` | changed the purpose to “{{value}}” | systemEventCopy | — | in-pr3 |
| `messages.system.changed-topic` | changed the topic to “{{value}}” | systemEventCopy | — | in-pr3 |
| `messages.system.channel-members_one` | {{count}} channel member | SystemMessageAvatars | — | in-pr3 |
| `messages.system.channel-members_other` | {{count}} channel members | SystemMessageAvatars | — | in-pr3 |
| `messages.system.cleared-purpose` | cleared the purpose | systemEventCopy | — | in-pr3 |
| `messages.system.cleared-topic` | cleared the topic | systemEventCopy | — | in-pr3 |
| `messages.system.created-channel` | created this channel | SystemMessageRow | — | in-pr3 |
| `messages.system.inline-you` | you | systemEventCopy | — | in-pr3 |
| `messages.system.joined` | joined | SystemMessageRow | — | in-pr3 |
| `messages.system.joined-along-with` | joined along with | SystemMessageRow | — | in-pr3 |
| `messages.system.joined-the-channel` | joined the channel | SystemMessageRow | — | in-pr3 |
| `messages.system.joined-then-left` | joined, then left the channel | SystemMessageRow | — | in-pr3 |
| `messages.system.left-the-channel` | left the channel | SystemMessageRow | — | in-pr3 |
| `messages.system.list-and` |  and  | SystemMessageRow | — | in-pr3 |
| `messages.system.list-and-comma` | , and  | SystemMessageRow | — | in-pr3 |
| `messages.system.list-separator` | ,  | SystemMessageRow | — | in-pr3 |
| `messages.system.others-connector` | , and | SystemMessageRow | — | in-pr3 |
| `messages.system.others-count_one` | {{count}} other | SystemMessageRow | — | in-pr3 |
| `messages.system.others-count_other` | {{count}} others | SystemMessageRow | — | in-pr3 |
| `messages.system.removed-a-message` | removed a message | SystemMessageRow | — | in-pr3 |
| `messages.system.removed-by-moderators` | Removed by community moderators | SystemMessageRow | — | in-pr3 |
| `messages.system.removed-prefix` | removed  | SystemMessageRow | — | in-pr3 |
| `messages.system.removed-suffix` |  from the channel | SystemMessageRow | — | in-pr3 |
| `messages.system.someone` | Someone | SystemMessageAvatars, SystemMessageRow | — | in-pr3 |
| `messages.system.unarchived-channel` | unarchived this channel | SystemMessageRow | — | in-pr3 |
| `messages.system.was-added` | was added | systemEventCopy | — | in-pr3 |
| `messages.system.were-added` | were added | systemEventCopy | — | in-pr3 |
| `messages.system.were-added-by` | were added by | systemEventCopy | — | in-pr3 |
| `messages.thread.collapse-replies` | Collapse replies | MessageThreadPanel | — | in-pr3 |
| `messages.thread.collapse-thread` | Collapse thread | MessageThreadPanel | — | in-pr3 |
| `messages.thread.composer-placeholder` | Reply in thread to {{author}} | MessageThreadPanel | — | in-pr3 |
| `messages.thread.composer-placeholder-huddle` | Message the huddle | MessageThreadPanel | — | in-pr3 |
| `messages.timeline.dm-intro-prefix` | This is the beginning of your direct message with | MessageTimeline | — | in-pr3 |
| `messages.timeline.dm-intro-suffix` | . | MessageTimeline | — | in-pr3 |
| `messages.timeline.empty-description` | Send the first message to start the thread. | MessageTimeline | — | in-pr3 |
| `messages.timeline.empty-title` | No messages yet | MessageTimeline | — | in-pr3 |
| `messages.timeline.jump-to-latest` | Jump to latest | MessageThreadPanel, MessageTimeline | — | in-pr3 |
| `messages.timeline.new-messages_one` | {{count}} new message | MessageThreadPanel, MessageTimeline | — | in-pr3 |
| `messages.timeline.new-messages_other` | {{count}} new messages | MessageThreadPanel, MessageTimeline | — | in-pr3 |
| `messages.voice.discard` | Discard voice note | VoiceNoteRecorder | — | in-pr3 |
| `messages.voice.finish-or-discard-first` | Finish or discard the voice note before attaching a file. | useComposerVoiceNote | — | in-pr3 |
| `messages.voice.interrupted` | The voice recording was interrupted. | useVoiceNoteRecorder | — | in-pr3 |
| `messages.voice.legend-preparing` | Preparing voice note | VoiceNoteRecorder | — | in-pr3 |
| `messages.voice.legend-recording` | Recording voice note | VoiceNoteRecorder | — | in-pr3 |
| `messages.voice.legend-requesting-access` | Waiting for microphone access | VoiceNoteRecorder | — | in-pr3 |
| `messages.voice.microphone-permission` | Allow Buzz to access your microphone to record a voice note. | useVoiceNoteRecorder | — | in-pr3 |
| `messages.voice.only-attachment` | A voice note must be the only attachment. | useComposerVoiceNote | — | in-pr3 |
| `messages.voice.prepare-failed` | Buzz could not prepare this voice note for upload. | useVoiceNoteRecorder | — | in-pr3 |
| `messages.voice.preparing-voice-note` | Preparing voice note… | VoiceNoteRecorder | — | in-pr3 |
| `messages.voice.recording-unavailable` | Voice recording is not available in this environment. | useVoiceNoteRecorder | — | in-pr3 |
| `messages.voice.start-failed` | Buzz could not start the voice recorder. | useVoiceNoteRecorder | — | in-pr3 |
| `messages.voice.waiting-for-microphone` | Waiting for microphone… | VoiceNoteRecorder | — | in-pr3 |

| `agent-memory.empty.description` | Try telling this agent to remember something for next time. | MemorySection | — | in-pr4 |
| `agent-memory.empty.title` | Build this agent's memory | MemorySection | — | in-pr4 |
| `agent-memory.entry.dangling-ref-tooltip` | This memory links to a slug that wasn't found in the loaded memory list. | MemorySection | — | in-pr4 |
| `agent-memory.entry.empty` | (empty) | MemorySection | — | in-pr4 |
| `agent-memory.entry.missing-links_one` | Missing link:  | MemorySection | — | in-pr4 |
| `agent-memory.entry.missing-links_other` | Missing links:  | MemorySection | — | in-pr4 |
| `agent-memory.graph.no-core-prefix` | No  | MemorySection | — | in-pr4 |
| `agent-memory.graph.no-core-suffix` |  memory yet — agent identity is unrooted. | MemorySection | — | in-pr4 |
| `agent-memory.graph.show-less` | Show less | MemorySection | — | in-pr4 |
| `agent-memory.graph.view-all_one` | View all ({{count}}) | MemorySection | — | in-pr4 |
| `agent-memory.graph.view-all_other` | View all ({{count}}) | MemorySection | — | in-pr4 |
| `agent-memory.section.error-title` | Couldn't load memory | MemorySection | — | in-pr4 |
| `agent-memory.section.loading` | Loading memory | MemorySection | — | in-pr4 |
| `agent-memory.section.refresh` | Refresh memory | MemorySection | — | in-pr4 |
| `agent-memory.section.refresh-failed` | Refresh failed. | MemorySection | — | in-pr4 |
| `agent-memory.section.retry` | Retry | MemorySection | — | in-pr4 |
| `agent-memory.section.retrying` | Retrying… | MemorySection | — | in-pr4 |
| `agent-memory.tooltip.truncated` | This list may be incomplete — the relay returned the maximum number of memories. | MemorySection | — | in-pr4 |
| `mesh-compute.card.advanced` | Advanced | MeshComputeSettingsCard | — | in-pr4 |
| `mesh-compute.card.check-error` | Couldn't check shared compute: {{error}} | MeshComputeSettingsCard | — | in-pr4 |
| `mesh-compute.card.debug-console` | Debug console: | MeshComputeSettingsCard | — | in-pr4 |
| `mesh-compute.card.max-vram` | Max VRAM (GB) | MeshComputeSettingsCard | — | in-pr4 |
| `mesh-compute.card.model` | Model | MeshComputeSettingsCard | — | in-pr4 |
| `mesh-compute.card.no-limit` | No limit | MeshComputeSettingsCard | — | in-pr4 |
| `mesh-compute.card.share-compute` | Share compute | MeshComputeSettingsCard | — | in-pr4 |
| `mesh-compute.card.share-compute-description` | Share this machine with members of this relay so they can run agents here. | MeshComputeSettingsCard | — | in-pr4 |
| `mesh-compute.card.share-this-machine` | Share this machine | MeshComputeSettingsCard | — | in-pr4 |
| `mesh-compute.card.sharing` | Sharing | MeshComputeSettingsCard | — | in-pr4 |
| `mesh-compute.card.status` | Status | MeshComputeSettingsCard | — | in-pr4 |
| `mesh-compute.download.downloading` | Downloading | MeshComputeSettingsCard | — | in-pr4 |
| `mesh-compute.download.preparing` | Preparing | MeshComputeSettingsCard | — | in-pr4 |
| `mesh-compute.model-fit.comfortable` | Fits well | MeshComputeSettingsCard | — | in-pr4 |
| `mesh-compute.model-fit.tight` | Tight fit | MeshComputeSettingsCard | — | in-pr4 |
| `mesh-compute.model-fit.too-large` | Too large | MeshComputeSettingsCard | — | in-pr4 |
| `mesh-compute.model-fit.trade-off` | Trade-off | MeshComputeSettingsCard | — | in-pr4 |
| `mesh-compute.model-option.advanced` | Advanced | MeshComputeSettingsCard | — | in-pr4 |
| `mesh-compute.model-option.custom` | Custom model… | MeshComputeSettingsCard | — | in-pr4 |
| `mesh-compute.model-option.installed` | Installed | MeshComputeSettingsCard | — | in-pr4 |
| `mesh-compute.model-option.recommended` | Recommended | MeshComputeSettingsCard | — | in-pr4 |
| `mesh-compute.model-picker.choose-or-enter` | Choose a model or enter a model reference or local file. | MeshComputeSettingsCard | — | in-pr4 |
| `mesh-compute.model-picker.custom-reference-label` | Custom model reference | MeshComputeSettingsCard | — | in-pr4 |
| `mesh-compute.model-picker.downloads-on-share` | Buzz downloads remote models when sharing starts. | MeshComputeSettingsCard | — | in-pr4 |
| `mesh-compute.model-picker.recommended` | Recommended for this machine. | MeshComputeSettingsCard | — | in-pr4 |
| `mesh-compute.model-picker.recommended-gpu` | Recommended for this machine ({{gpuName}}, {{vramDisplay}} AI memory). | MeshComputeSettingsCard | — | in-pr4 |
| `mesh-compute.model-picker.select-placeholder` | Select a model | MeshComputeSettingsCard | — | in-pr4 |
| `mesh-compute.serving-indicator.idle-right-now` | Idle · no one using it right now | servingUsage | — | in-pr4 |
| `mesh-compute.serving-indicator.idle-yet` | Idle · no one using it yet | servingUsage | — | in-pr4 |
| `mesh-compute.serving-indicator.local-active` | Serving your agent · {{inflight}} live | servingUsage | — | in-pr4 |
| `mesh-compute.serving-indicator.peers-detail_one` | {{count}} peer on the mesh · {{tokensPerSecond}} tok/s | servingUsage | — | in-pr4 |
| `mesh-compute.serving-indicator.peers-detail_other` | {{count}} peers on the mesh · {{tokensPerSecond}} tok/s | servingUsage | — | in-pr4 |
| `mesh-compute.serving-indicator.remote-active` | In use now by another member · {{inflight}} live | servingUsage | — | in-pr4 |
| `mesh-compute.serving-indicator.remote-idle_one` | Used by another member · {{count}} request | servingUsage | — | in-pr4 |
| `mesh-compute.serving-indicator.remote-idle_other` | Used by another member · {{count}} requests | servingUsage | — | in-pr4 |
| `mesh-compute.serving-indicator.served-this-session_one` | {{count}} request served this session | servingUsage | — | in-pr4 |
| `mesh-compute.serving-indicator.served-this-session_other` | {{count}} requests served this session | servingUsage | — | in-pr4 |
| `mesh-compute.serving-indicator.tokens-per-second` | {{tokensPerSecond}} tok/s | servingUsage | — | in-pr4 |
| `mesh-compute.status.active` | Active. {{reason}} | MeshComputeSettingsCard | — | in-pr4 |
| `mesh-compute.status.active-with-model` | Active — {{modelLabel}}. {{reason}} | MeshComputeSettingsCard | — | in-pr4 |
| `mesh-compute.status.checking` | Checking status… | MeshComputeSettingsCard | — | in-pr4 |
| `mesh-compute.status.consuming` | This machine is currently using another member's shared compute. Turn on sharing to switch to the selected local model; Buzz may briefly restart. | MeshComputeSettingsCard | — | in-pr4 |
| `mesh-compute.status.could-not-load` | Couldn't load: {{reason}} | MeshComputeSettingsCard | — | in-pr4 |
| `mesh-compute.status.could-not-start` | Couldn't start. | MeshComputeSettingsCard | — | in-pr4 |
| `mesh-compute.status.model-with-relay-members` | {{modelLabel}} with relay members. | MeshComputeSettingsCard | — | in-pr4 |
| `mesh-compute.status.sharing` | Sharing with relay members. | MeshComputeSettingsCard | — | in-pr4 |
| `mesh-compute.status.sharing-with-model` | Sharing {{modelLabel}} with relay members. | MeshComputeSettingsCard | — | in-pr4 |
| `mesh-compute.status.starting` | Starting… | MeshComputeSettingsCard | — | in-pr4 |
| `mesh-compute.status.stopping` | Stopping… | MeshComputeSettingsCard | — | in-pr4 |
| `mesh-compute.status.with-relay-members` |  with relay members. | MeshComputeSettingsCard | — | in-pr4 |
| `moderation.composer.timed-out-remaining` | You're timed out by community moderators — {{remaining}} left. | ComposerTimeoutBanner | — | in-pr4 |
| `moderation.menu.author-banned` | Author banned | MessageModerationMenuItems | — | in-pr4 |
| `moderation.menu.author-removed` | Author removed from channel | MessageModerationMenuItems | — | in-pr4 |
| `moderation.menu.author-timed-out` | Author timed out | MessageModerationMenuItems | — | in-pr4 |
| `moderation.menu.ban-author` | Ban author from community | MessageModerationMenuItems | — | in-pr4 |
| `moderation.menu.ban-lifted` | Ban lifted | MessageModerationMenuItems | — | in-pr4 |
| `moderation.menu.kick-from-channel` | Kick from channel | MessageModerationMenuItems | — | in-pr4 |
| `moderation.menu.lift-ban` | Lift ban | MessageModerationMenuItems | — | in-pr4 |
| `moderation.menu.time-out-author` | Time out author | MessageModerationMenuItems | — | in-pr4 |
| `moderation.menu.timeout-lifted` | Timeout lifted | MessageModerationMenuItems | — | in-pr4 |
| `moderation.report.additional-context` | Additional context (optional) | ReportMessageDialog | — | in-pr4 |
| `moderation.report.cancel` | Cancel | ReportMessageDialog | — | in-pr4 |
| `moderation.report.description` | Reports go to this community's moderators for review. The author is not notified of who reported them. | ReportMessageDialog | — | in-pr4 |
| `moderation.report.illegal-content` | Illegal content | ReportMessageDialog | — | in-pr4 |
| `moderation.report.impersonation` | Impersonation | ReportMessageDialog | — | in-pr4 |
| `moderation.report.malware-or-scam` | Malware or scam | ReportMessageDialog | — | in-pr4 |
| `moderation.report.note-placeholder` | Add anything that helps moderators... | ReportMessageDialog | — | in-pr4 |
| `moderation.report.nudity-or-sexual-content` | Nudity or sexual content | ReportMessageDialog | — | in-pr4 |
| `moderation.report.other` | Other | ReportMessageDialog | — | in-pr4 |
| `moderation.report.profanity-or-hate-speech` | Profanity or hate speech | ReportMessageDialog | — | in-pr4 |
| `moderation.report.spam` | Spam | ReportMessageDialog | — | in-pr4 |
| `moderation.report.submit` | Submit report | ReportMessageDialog | — | in-pr4 |
| `moderation.report.submit-failed` | Failed to submit report | ReportMessageDialog | — | in-pr4 |
| `moderation.report.submitted` | Report submitted to community moderators | ReportMessageDialog | — | in-pr4 |
| `reminders.dialog.cancel` | Cancel | RemindMeLaterDialog | — | in-pr4 |
| `reminders.dialog.custom-heading` | Custom date & time | RemindMeLaterDialog | — | in-pr4 |
| `reminders.dialog.date-label` | Reminder date | RemindMeLaterDialog | — | in-pr4 |
| `reminders.dialog.description` | Choose when you want to be reminded about this message. | RemindMeLaterDialog | — | in-pr4 |
| `reminders.dialog.note-label` | Note (optional) | RemindMeLaterDialog | — | in-pr4 |
| `reminders.dialog.note-placeholder` | Add a note... | RemindMeLaterDialog | — | in-pr4 |
| `reminders.dialog.submit` | Set reminder | RemindMeLaterDialog | — | in-pr4 |
| `reminders.dialog.time-label` | Reminder time | RemindMeLaterDialog | — | in-pr4 |
| `reminders.dialog.title` | Remind me later | RemindMeLaterDialog | — | in-pr4 |
| `reminders.group.completed` | Completed | reminderFilters | — | in-pr4 |
| `reminders.group.overdue` | Overdue | reminderFilters | — | in-pr4 |
| `reminders.group.today` | Today | reminderFilters | — | in-pr4 |
| `reminders.group.upcoming` | Upcoming | reminderFilters | — | in-pr4 |
| `reminders.notification.body-fallback` | A reminder is waiting | useReminderNotifications | — | in-pr4 |
| `reminders.notification.due-count_one` | {{count}} reminder is due | useReminderNotifications | — | in-pr4 |
| `reminders.notification.due-count_other` | {{count}} reminders are due | useReminderNotifications | — | in-pr4 |
| `reminders.notification.title` | Reminder due | useReminderNotifications | — | in-pr4 |
| `reminders.panel.back` | Back to reminders | RemindersPanel | — | in-pr4 |
| `reminders.panel.cancel` | Cancel | RemindersPanel | — | in-pr4 |
| `reminders.panel.complete` | Complete | RemindersPanel | — | in-pr4 |
| `reminders.panel.empty` | No reminders | RemindersPanel | — | in-pr4 |
| `reminders.panel.empty-hint` | Use "Remind me later" on any message to create one. | RemindersPanel | — | in-pr4 |
| `reminders.panel.loading` | Loading reminders... | RemindersPanel | — | in-pr4 |
| `reminders.panel.note-heading` | Note | RemindersPanel | — | in-pr4 |
| `reminders.panel.open-message` | Open message | RemindersPanel | — | in-pr4 |
| `reminders.panel.select-one` | Select a reminder | RemindersPanel | — | in-pr4 |
| `reminders.panel.title` | Reminder | RemindersPanel | — | in-pr4 |
| `reminders.preset.in-1-hour` | In 1 hour | timePresets | — | in-pr4 |
| `reminders.preset.in-3-hours` | In 3 hours | timePresets | — | in-pr4 |
| `reminders.preset.in-30-minutes` | In 30 minutes | timePresets | — | in-pr4 |
| `reminders.preset.next-monday-9am` | Next Monday at 9am | timePresets | — | in-pr4 |
| `reminders.preset.tomorrow-9am` | Tomorrow at 9am | timePresets | — | in-pr4 |
| `reminders.relative.days-from-now` | in {{days}}d | RemindersPanel | — | in-pr4 |
| `reminders.relative.days-overdue` | {{days}}d overdue | RemindersPanel | — | in-pr4 |
| `reminders.relative.hours-from-now` | in {{hours}}h | RemindersPanel | — | in-pr4 |
| `reminders.relative.hours-overdue` | {{hours}}h overdue | RemindersPanel | — | in-pr4 |
| `reminders.relative.just-now` | just now | RemindersPanel | — | in-pr4 |
| `reminders.relative.minutes-from-now` | in {{minutes}}m | RemindersPanel | — | in-pr4 |
| `reminders.relative.minutes-overdue` | {{minutes}}m overdue | RemindersPanel | — | in-pr4 |
| `reminders.relative.within-a-minute` | in less than a minute | RemindersPanel | — | in-pr4 |
| `reminders.row.in` | in | RemindersPanel | — | in-pr4 |
| `reminders.snooze.action` | Snooze | SnoozeMenu | — | in-pr4 |
| `reminders.snooze.custom` | Custom… | SnoozeMenu | — | in-pr4 |
| `reminders.snooze.date-label` | Snooze date | SnoozeMenu | — | in-pr4 |
| `reminders.snooze.time-label` | Snooze time | SnoozeMenu | — | in-pr4 |
| `reminders.snooze.until` | Snooze until | SnoozeMenu | — | in-pr4 |
| `reminders.source.dm-location` | DM with {{name}} | RemindersPanel | — | in-pr4 |
| `reminders.source.unknown-channel` | Unknown channel | RemindersPanel | — | in-pr4 |
| `reminders.toast.cancel-failed` | Failed to cancel reminder | RemindersPanel | — | in-pr4 |
| `reminders.toast.cancelled` | Reminder cancelled | RemindersPanel | — | in-pr4 |
| `reminders.toast.complete-failed` | Failed to complete reminder | RemindersPanel | — | in-pr4 |
| `reminders.toast.completed` | Reminder completed | RemindersPanel | — | in-pr4 |
| `reminders.toast.create-failed` | Failed to create reminder | RemindMeLaterDialog | — | in-pr4 |
| `reminders.toast.created` | Reminder set | RemindMeLaterDialog | — | in-pr4 |
| `reminders.toast.snooze-failed` | Failed to snooze reminder | RemindersPanel | — | in-pr4 |
| `reminders.toast.snoozed` | Reminder snoozed | RemindersPanel | — | in-pr4 |
| `status.dialog.choose-emoji` | Choose a status emoji | SetStatusDialog | — | in-pr4 |
| `status.dialog.clear-status` | Clear status | SetStatusDialog | — | in-pr4 |
| `status.dialog.duration` | Duration | SetStatusDialog | — | in-pr4 |
| `status.dialog.expiration-date` | Status expiration date | SetStatusDialog | — | in-pr4 |
| `status.dialog.expiration-time` | Status expiration time | SetStatusDialog | — | in-pr4 |
| `status.dialog.future-duration` | Choose a duration in the future. | SetStatusDialog | — | in-pr4 |
| `status.dialog.placeholder` | What’s your status? | SetStatusDialog | — | in-pr4 |
| `status.dialog.quick-statuses` | Quick statuses | SetStatusDialog | — | in-pr4 |
| `status.dialog.save-status` | Save status | SetStatusDialog | — | in-pr4 |
| `status.dialog.subtitle` | Let others know what you're up to. | SetStatusDialog | — | in-pr4 |
| `status.dialog.title` | Set a status | SetStatusDialog | — | in-pr4 |
| `status.dialog.until` | Until | SetStatusDialog | — | in-pr4 |
| `status.duration.custom` | Custom | SetStatusDialog | — | in-pr4 |
| `status.duration.eight-hours` | 8 hours | SetStatusDialog | — | in-pr4 |
| `status.duration.one-hour` | 1 hour | SetStatusDialog | — | in-pr4 |
| `status.duration.this-week` | This week | SetStatusDialog | — | in-pr4 |
| `status.duration.today` | Today | SetStatusDialog | — | in-pr4 |
| `status.indicator.in-huddle` | In a huddle | UserNameIndicators | — | in-pr4 |
| `status.indicator.in-huddle-aria` | 🎧 In a huddle | UserNameIndicators | — | in-pr4 |
| `status.indicator.status-set` | Status set | UserNameIndicators | — | in-pr4 |
| `status.indicator.user-status` | User status | UserNameIndicators | — | in-pr4 |

| `huddle.agent.add-failed` | Failed to add agent: {{msg}} | HuddleBar | — | in-pr4 |
| `huddle.agent.choose-voice` | Choose agent voice | AgentVoiceMenu | — | in-pr4 |
| `huddle.agent.text-to-speech` | Agent text-to-speech | AgentVoiceMenu | — | in-pr4 |
| `huddle.agent.voice` | Agent voice | AgentVoiceMenu | — | in-pr4 |
| `huddle.agent.voice-unavailable` | Unavailable | AgentVoiceMenu | — | in-pr4 |
| `huddle.agent.voice-update-failed` | Agent voice could not be updated. | AgentVoiceMenu | — | in-pr4 |
| `huddle.attachment.join` | Join | HuddleAttachment | — | in-pr4 |
| `huddle.attachment.joining` | Joining | HuddleAttachment | — | in-pr4 |
| `huddle.attachment.participant-count_one` | {{count}} participant | HuddleAttachment | — | in-pr4 |
| `huddle.attachment.participant-count_other` | {{count}} participants | HuddleAttachment | — | in-pr4 |
| `huddle.attachment.status-active` | In progress | HuddleAttachment | — | in-pr4 |
| `huddle.attachment.status-ended` | Ended | HuddleAttachment | — | in-pr4 |
| `huddle.attachment.title` | Huddle | HuddleAttachment | — | in-pr4 |
| `huddle.attachment.unavailable-body` | This huddle card is missing session details. | HuddleAttachment | — | in-pr4 |
| `huddle.attachment.unavailable-title` | Huddle unavailable | HuddleAttachment | — | in-pr4 |
| `huddle.attachment.view` | View huddle | HuddleAttachment | — | in-pr4 |
| `huddle.attachment.view-short` | View | HuddleAttachment | — | in-pr4 |
| `huddle.bar.add-agent` | Add agent to huddle | HuddleBar | — | in-pr4 |
| `huddle.bar.add-agent-short` | Add agent | HuddleBar | — | in-pr4 |
| `huddle.bar.dismiss-error` | Dismiss error | HuddleBar | — | in-pr4 |
| `huddle.bar.leave` | Leave huddle | HuddleBar | — | in-pr4 |
| `huddle.bar.leave-short` | Leave | HuddleBar | — | in-pr4 |
| `huddle.bar.open-window` | Open huddle in a new window | HuddleBar | — | in-pr4 |
| `huddle.bar.open-window-short` | Open huddle window | HuddleBar | — | in-pr4 |
| `huddle.bar.reaction-failed` | Reaction failed | HuddleBar | — | in-pr4 |
| `huddle.bar.reactions` | Emoji reactions | HuddleBar | — | in-pr4 |
| `huddle.bar.return-to-drawer` | Return huddle to drawer | HuddleBar | — | in-pr4 |
| `huddle.bar.start-transcript` | Start transcript | HuddleBar | — | in-pr4 |
| `huddle.bar.stop-transcript` | Stop transcript | HuddleBar | — | in-pr4 |
| `huddle.bar.transcript-failed` | Transcript failed: {{message}} | HuddleBar | — | in-pr4 |
| `huddle.device.change-hint` | Change takes effect on next huddle | MicControls | — | in-pr4 |
| `huddle.device.mic-fallback` | Mic {{id}} | MicControls | — | in-pr4 |
| `huddle.device.microphone` | Microphone | MicControls | — | in-pr4 |
| `huddle.device.speaker` | Speaker | MicControls | — | in-pr4 |
| `huddle.device.system-default` | System default | MicControls | — | in-pr4 |
| `huddle.error.audio-unavailable` | Huddle audio isn’t available on this server. Ask an administrator to turn it on. | huddleError | — | in-pr4 |
| `huddle.error.join-failed` | Couldn’t join the huddle. | huddleError | — | in-pr4 |
| `huddle.error.start-failed` | Couldn’t start the huddle. | huddleError | — | in-pr4 |
| `huddle.indicator.active-tooltip_one` | Huddle active — {{count}} participant | HuddleIndicator | — | in-pr4 |
| `huddle.indicator.active-tooltip_other` | Huddle active — {{count}} participants | HuddleIndicator | — | in-pr4 |
| `huddle.indicator.join` | Join huddle | HuddleIndicator | — | in-pr4 |
| `huddle.indicator.join-active_one` | Join active huddle ({{count}} participant) | HuddleIndicator | — | in-pr4 |
| `huddle.indicator.join-active_other` | Join active huddle ({{count}} participants) | HuddleIndicator | — | in-pr4 |
| `huddle.indicator.start` | Start huddle | HuddleIndicator | — | in-pr4 |
| `huddle.indicator.tooltip` | Huddle | HuddleIndicator | — | in-pr4 |
| `huddle.mic.click-to-unmute` | Click to unmute or hold | MicControls | — | in-pr4 |
| `huddle.mic.continuous` | Microphone is continuous. | MicControls | — | in-pr4 |
| `huddle.mic.input-mode` | Input Mode | MicControls | — | in-pr4 |
| `huddle.mic.input-volume` | Input Volume | MicControls | — | in-pr4 |
| `huddle.mic.mute` | Mute microphone | MicControls | — | in-pr4 |
| `huddle.mic.open-settings` | Open Settings | MicControls | — | in-pr4 |
| `huddle.mic.permission-hint` | Check app microphone permission or select another input device. | MicControls | — | in-pr4 |
| `huddle.mic.ptt-enabled` | Push to Talk is enabled. | MicControls | — | in-pr4 |
| `huddle.mic.ptt-off` | Turn off Push to Talk | MicControls | — | in-pr4 |
| `huddle.mic.ptt-on` | Turn on Push to Talk | MicControls | — | in-pr4 |
| `huddle.mic.push-to-talk` | Push to Talk | MicControls | — | in-pr4 |
| `huddle.mic.settings` | Audio settings | MicControls | — | in-pr4 |
| `huddle.mic.unavailable` | Microphone unavailable | MicControls | — | in-pr4 |
| `huddle.mic.unavailable-detail` | Microphone unavailable. Check app permissions or input device. | MicControls | — | in-pr4 |
| `huddle.mic.unmute` | Unmute microphone | MicControls | — | in-pr4 |
| `huddle.model.both` | Voice models: STT {{stt}}, TTS {{tts}} | HuddleBar | — | in-pr4 |
| `huddle.model.stt` | STT model: {{stt}} | HuddleBar | — | in-pr4 |
| `huddle.model.tts` | TTS model: {{tts}} | HuddleBar | — | in-pr4 |
| `huddle.participant.fallback-agent` | Agent {{key}} | ParticipantList | — | in-pr4 |
| `huddle.participant.fallback-participant` | Participant {{key}} | ParticipantList | — | in-pr4 |
| `huddle.participants.count_one` | {{count}} participant | ParticipantList | — | in-pr4 |
| `huddle.participants.count_other` | {{count}} participants | ParticipantList | — | in-pr4 |
| `huddle.participants.heading` | Participants | ParticipantList | — | in-pr4 |
| `huddle.participants.open-profile` | Open profile for {{name}} | ParticipantList | — | in-pr4 |
| `huddle.participants.remove-agent` | Remove {{name}} from huddle | ParticipantList | — | in-pr4 |
| `huddle.participants.remove-agent-confirm` | Remove this agent from the huddle? | HuddleBar | — | in-pr4 |
| `huddle.participants.show-all` | Show all huddle participants ({{count}}) | ParticipantList | — | in-pr4 |
| `huddle.participants.status-agent` | Agent | ParticipantList | — | in-pr4 |
| `huddle.participants.status-in-huddle` | In huddle | ParticipantList | — | in-pr4 |
| `huddle.participants.status-speaking` | Speaking | ParticipantList | — | in-pr4 |
| `huddle.participants.stop` | Stop | ParticipantList | — | in-pr4 |
| `huddle.participants.stop-agent` | Stop {{name}} speaking | ParticipantList | — | in-pr4 |
| `huddle.participants.voice-settings` | Voice settings for {{name}} | AgentVoiceMenu, ParticipantList | — | in-pr4 |
| `huddle.speaker.got-it` | Got it | MicControls | — | in-pr4 |
| `huddle.speaker.headphones-aria` | Headphones recommended | MicControls | — | in-pr4 |
| `huddle.speaker.headphones-body` | If people are nearby, speakers can feed back into your mic. Headphones keep huddles clearer. | MicControls | — | in-pr4 |
| `huddle.speaker.headphones-title` | Headphones help prevent echo | MicControls | — | in-pr4 |
| `huddle.speaker.mute-agent` | Mute agent speech | MicControls | — | in-pr4 |
| `huddle.speaker.settings` | Speaker settings | MicControls | — | in-pr4 |
| `huddle.speaker.unmute-agent` | Unmute agent speech | MicControls | — | in-pr4 |
| `huddle.sr.mic-connected` | In huddle, microphone connected | HuddleBar | — | in-pr4 |
| `huddle.sr.mic-missing` | In huddle, no microphone | HuddleBar | — | in-pr4 |
| `huddle.sr.stt-model` | , STT model {{stt}} | HuddleBar | — | in-pr4 |
| `huddle.sr.tts-model` | , TTS model {{tts}} | HuddleBar | — | in-pr4 |
| `huddle.sr.voice-input` | , voice input: {{mode}} | HuddleBar | — | in-pr4 |
| `huddle.sr.voice-input-ptt` | push to talk, press Ctrl+Space to transmit | HuddleBar | — | in-pr4 |
| `huddle.sr.voice-input-vad` | voice activity detection | HuddleBar | — | in-pr4 |
| `huddle.starting` | Starting huddle | HuddleStartingView | — | in-pr4 |
| `huddle.transcript-intro.body` | Chat with huddle participants and agents. The transcript appears here too. | HuddleTranscriptIntro | — | in-pr4 |
| `huddle.transcript-intro.title` | Huddle chat | HuddleTranscriptIntro | — | in-pr4 |

| `members.add.add-member` | Add member | AddMemberDialog | — | in-pr4 |
| `members.add.admin-added_one` | Admin added | AddMemberDialog | — | in-pr4 |
| `members.add.admin-added_other` | Admins added | AddMemberDialog | — | in-pr4 |
| `members.add.already-member` | This person is already a community member. | AddMemberDialog | — | in-pr4 |
| `members.add.choose-role` | Choose member role | AddMemberDialog | — | in-pr4 |
| `members.add.description` | Add a person to this community by their public key. | AddMemberDialog | — | in-pr4 |
| `members.add.inviting` | Inviting… | AddMemberDialog | — | in-pr4 |
| `members.add.member-added_one` | Member added | AddMemberDialog | — | in-pr4 |
| `members.add.member-added_other` | Members added | AddMemberDialog | — | in-pr4 |
| `members.add.no-people-found` | No people found. Paste a full npub or hex public key to add someone directly. | AddMemberDialog | — | in-pr4 |
| `members.add.person` | Person | AddMemberDialog | — | in-pr4 |
| `members.add.public-key-hint` | public key | AddMemberDialog | — | in-pr4 |
| `members.add.search-placeholder` | Search people or paste an npub | AddMemberDialog | — | in-pr4 |
| `members.add.searching` | Searching… | AddMemberDialog | — | in-pr4 |
| `members.card.actions` | Actions | CommunityMembersCard | — | in-pr4 |
| `members.card.add-member` | Add Member | CommunityMembersCard | — | in-pr4 |
| `members.card.change-role-failed` | Failed to change role | CommunityMembersCard | — | in-pr4 |
| `members.card.description` | Manage who has access to this relay. | CommunityMembersCard | — | in-pr4 |
| `members.card.empty` | No members yet. | CommunityMembersCard | — | in-pr4 |
| `members.card.joined` | Joined {{when}} | CommunityMembersCard | — | in-pr4 |
| `members.card.make-admin` | Make Admin | CommunityMembersCard | — | in-pr4 |
| `members.card.make-member` | Make Member | CommunityMembersCard | — | in-pr4 |
| `members.card.remove` | Remove | CommunityMembersCard | — | in-pr4 |
| `members.card.role-changed` | Role changed to {{role}} | CommunityMembersCard | — | in-pr4 |
| `members.card.title` | Community Members | CommunityMembersCard | — | in-pr4 |
| `members.card.you` | (you) | CommunityMembersCard | — | in-pr4 |
| `members.invite.choose-expiry` | Choose invite expiry | InviteLinkSection | — | in-pr4 |
| `members.invite.choose-max-uses` | Choose maximum invite uses | InviteLinkSection | — | in-pr4 |
| `members.invite.copied` | Copied | InviteLinkSection | — | in-pr4 |
| `members.invite.copied-toast` | Invite link copied | InviteLinkSection | — | in-pr4 |
| `members.invite.copy-failed` | Couldn’t copy the invite link. Try again. | InviteLinkSection | — | in-pr4 |
| `members.invite.copy-link` | Copy link | InviteLinkSection | — | in-pr4 |
| `members.invite.create-failed` | Couldn’t create an invite link. | InviteLinkSection | — | in-pr4 |
| `members.invite.create-failed-placeholder` | Couldn’t create invite link | InviteLinkSection | — | in-pr4 |
| `members.invite.creating` | Creating invite link… | InviteLinkSection | — | in-pr4 |
| `members.invite.description` | Add someone directly or share a link they can use to join. | CommunityInviteDialog | — | in-pr4 |
| `members.invite.expires-after` | Expires after | InviteLinkSection | — | in-pr4 |
| `members.invite.limit-uses` | Limit number of uses | InviteLinkSection | — | in-pr4 |
| `members.invite.link-label` | Community invite link | InviteLinkSection | — | in-pr4 |
| `members.invite.no-limit` | No limit | InviteLinkSection | — | in-pr4 |
| `members.invite.or-copy-link` | Or, copy a link | CommunityInviteDialog | — | in-pr4 |
| `members.invite.retry` | Retry | InviteLinkSection | — | in-pr4 |
| `members.invite.submit` | Invite | CommunityInviteDialog | — | in-pr4 |
| `members.invite.title` | Invite to community | CommunityInviteDialog | — | in-pr4 |
| `members.invite.ttl-1-day` | 1 day | InviteLinkSection | — | in-pr4 |
| `members.invite.ttl-3-days` | 3 days | InviteLinkSection | — | in-pr4 |
| `members.invite.ttl-30-days` | 30 days | InviteLinkSection | — | in-pr4 |
| `members.invite.ttl-7-days` | 7 days | InviteLinkSection | — | in-pr4 |
| `members.invite.uses-1` | 1 use | InviteLinkSection | — | in-pr4 |
| `members.invite.uses-10` | 10 uses | InviteLinkSection | — | in-pr4 |
| `members.invite.uses-25` | 25 uses | InviteLinkSection | — | in-pr4 |
| `members.invite.uses-3` | 3 uses | InviteLinkSection | — | in-pr4 |
| `members.invite.uses-5` | 5 uses | InviteLinkSection | — | in-pr4 |
| `members.join-alerts.body-joined` | {{name}} joined | joinAlerts | — | in-pr4 |
| `members.join-alerts.summary-body` | {{count}} new members joined | joinAlerts | — | in-pr4 |
| `members.join-alerts.title` | New community member | joinAlerts | — | in-pr4 |
| `members.join-alerts.title-in-community` | New member in {{community}} | joinAlerts | — | in-pr4 |
| `members.remove.cancel` | Cancel | ConfirmRemoveDialog | — | in-pr4 |
| `members.remove.confirm` | Remove | ConfirmRemoveDialog | — | in-pr4 |
| `members.remove.description` | This will immediately revoke their access to the relay. | ConfirmRemoveDialog | — | in-pr4 |
| `members.remove.failed` | Failed to remove member | ConfirmRemoveDialog | — | in-pr4 |
| `members.remove.removed` | Member removed | ConfirmRemoveDialog | — | in-pr4 |
| `members.remove.removing` | Removing... | ConfirmRemoveDialog | — | in-pr4 |
| `members.remove.title` | Remove {{name}}? | ConfirmRemoveDialog | — | in-pr4 |
| `members.settings.actions-for` | Actions for {{name}} | CommunityMembersSettingsCard | — | in-pr4 |
| `members.settings.added` | Added {{date}} | CommunityMembersSettingsCard | — | in-pr4 |
| `members.settings.checking-invite-permissions` | Checking invite permissions… | CommunityMembersSettingsCard | — | in-pr4 |
| `members.settings.community-owner` | Community owner | CommunityMembersSettingsCard | — | in-pr4 |
| `members.settings.empty` | No community members yet. | CommunityMembersSettingsCard | — | in-pr4 |
| `members.settings.invite-trigger` | Invite to community | CommunityMembersSettingsCard | — | in-pr4 |
| `members.settings.invites` | Invites | CommunityMembersSettingsCard | — | in-pr4 |
| `members.settings.invites-description` | Manage members and community access. | CommunityMembersSettingsCard | — | in-pr4 |
| `members.settings.loading` | Loading community members… | CommunityMembersSettingsCard | — | in-pr4 |
| `members.settings.made-community-admin` | Made community admin | CommunityMembersSettingsCard | — | in-pr4 |
| `members.settings.made-community-member` | Made community member | CommunityMembersSettingsCard | — | in-pr4 |
| `members.settings.make-admin` | Make admin | CommunityMembersSettingsCard | — | in-pr4 |
| `members.settings.make-member` | Make member | CommunityMembersSettingsCard | — | in-pr4 |
| `members.settings.members` | Members | CommunityMembersSettingsCard | — | in-pr4 |
| `members.settings.no-search-results` | No members match your search. | CommunityMembersSettingsCard | — | in-pr4 |
| `members.settings.open-profile` | Open profile for {{name}} | CommunityMembersSettingsCard | — | in-pr4 |
| `members.settings.remove-from-community` | Remove from community | CommunityMembersSettingsCard | — | in-pr4 |
| `members.settings.removed-community-member` | Removed community member | CommunityMembersSettingsCard | — | in-pr4 |
| `members.settings.search-placeholder` | Search members | CommunityMembersSettingsCard | — | in-pr4 |
| `members.settings.unnamed-member` | Unnamed member | CommunityMembersSettingsCard | — | in-pr4 |
| `members.settings.update-failed` | Couldn’t update this community member. | CommunityMembersSettingsCard | — | in-pr4 |
| `members.settings.you` | You | CommunityMembersSettingsCard | — | in-pr4 |

| `projects.breadcrumb.detail-nav` | Project breadcrumb | ProjectDetailChrome | — | in-pr4 |
| `projects.breadcrumb.workspace-nav` | Projects breadcrumb | ProjectDetailChrome | — | in-pr4 |
| `projects.card.access-failed` | Access failed | ProjectCards | — | in-pr4 |
| `projects.card.access-failed-description` | Buzz could not authenticate with this repository. | ProjectCards | — | in-pr4 |
| `projects.card.branch-missing` | Branch missing | ProjectCards | — | in-pr4 |
| `projects.card.branch-missing-description` | The advertised branch is missing from the git remote. | ProjectCards | — | in-pr4 |
| `projects.card.cancel` | Cancel | ProjectCards | — | in-pr4 |
| `projects.card.create-project` | Create project | ProjectCards | — | in-pr4 |
| `projects.card.default-description` | A shared space for internal git work. | ProjectCards | — | in-pr4 |
| `projects.card.delete-project` | Delete project | ProjectCards | — | in-pr4 |
| `projects.card.delete-project-body` | Delete {{name}} from Projects for everyone. This can only be done for projects you own and cannot be undone. | ProjectCards | — | in-pr4 |
| `projects.card.delete-project-question` | Delete project? | ProjectCards | — | in-pr4 |
| `projects.card.deleting` | Deleting... | ProjectCards | — | in-pr4 |
| `projects.card.empty-body` | Projects published to this relay will appear here. | ProjectCards | — | in-pr4 |
| `projects.card.empty-filtered-body` | Try another owner filter or sort mode. | ProjectCards | — | in-pr4 |
| `projects.card.empty-filtered-title` | No matching projects | ProjectCards | — | in-pr4 |
| `projects.card.empty-title` | No projects yet | ProjectCards | — | in-pr4 |
| `projects.card.more-options` | More options for {{name}} | ProjectCards | — | in-pr4 |
| `projects.card.no-access` | No access | ProjectCards | — | in-pr4 |
| `projects.card.no-access-channel` | No access channel | ProjectCards | — | in-pr4 |
| `projects.card.no-access-channel-description` | The repository has no access channel binding, so the relay cannot authorize reads. | ProjectCards | — | in-pr4 |
| `projects.card.no-access-description` | You’re not a member of the channel that grants access to this repository. | ProjectCards | — | in-pr4 |
| `projects.card.repository-status-aria` | Repository {{status}} | ProjectCards | — | in-pr4 |
| `projects.card.stat-commits_one` | commit | ProjectCards | — | in-pr4 |
| `projects.card.stat-commits_other` | commits | ProjectCards | — | in-pr4 |
| `projects.card.stat-reviews_one` | review | ProjectCards | — | in-pr4 |
| `projects.card.stat-reviews_other` | reviews | ProjectCards | — | in-pr4 |
| `projects.card.stat-tasks_one` | task | ProjectCards | — | in-pr4 |
| `projects.card.stat-tasks_other` | tasks | ProjectCards | — | in-pr4 |
| `projects.card.unavailable` | Unavailable | ProjectCards | — | in-pr4 |
| `projects.card.unavailable-description` | Buzz could not load this repository. | ProjectCards | — | in-pr4 |
| `projects.card.uninitialized` | Uninitialized | ProjectCards | — | in-pr4 |
| `projects.card.uninitialized-description` | No git repository was found on the Buzz relay. | ProjectCards | — | in-pr4 |
| `projects.card.unreachable` | Unreachable | ProjectCards | — | in-pr4 |
| `projects.card.unreachable-description` | The Buzz git service could not be reached. | ProjectCards | — | in-pr4 |
| `projects.card.view-project` | View {{name}} | ProjectCards | — | in-pr4 |
| `projects.clone-error.access-required-github` | This repository requires GitHub authentication. Buzz currently clones public GitHub repositories without credentials. | projectGitError | — | in-pr4 |
| `projects.clone-error.access-required-title` | Repository access required | projectGitError | — | in-pr4 |
| `projects.clone-error.access-restricted-description` | You need access to the repository’s channel before you can clone it. | projectGitError | — | in-pr4 |
| `projects.clone-error.fallback-description` | Try again. If the problem continues, contact the repository owner. | projectGitError | — | in-pr4 |
| `projects.clone-error.fallback-github` | Try again, or open the repository on GitHub for more information. | projectGitError | — | in-pr4 |
| `projects.clone-error.fallback-title` | Couldn’t clone repository | projectGitError | — | in-pr4 |
| `projects.clone-error.folder-exists-description` | Choose a different repositories directory or remove the existing checkout. | projectGitError | — | in-pr4 |
| `projects.clone-error.folder-exists-title` | Local folder already exists | projectGitError | — | in-pr4 |
| `projects.clone-error.not-found-description` | Check that the repository link is correct and that the repository still exists. | projectGitError | — | in-pr4 |
| `projects.clone-error.not-found-title` | Repository not found | projectGitError | — | in-pr4 |
| `projects.clone-error.unreachable-description` | Check your connection and try cloning again. | projectGitError | — | in-pr4 |
| `projects.clone-error.unreachable-title` | Couldn’t reach the repository | projectGitError | — | in-pr4 |
| `projects.digest.active-projects_one` | {{count}} active project | projectsActivityDigest | — | in-pr4 |
| `projects.digest.active-projects_other` | {{count}} active projects | projectsActivityDigest | — | in-pr4 |
| `projects.digest.join-and` | , and  | ProjectsOverviewPanel | — | in-pr4 |
| `projects.digest.join-comma` | ,  | ProjectsOverviewPanel | — | in-pr4 |
| `projects.digest.new-commits_one` | {{count}} new commit | projectsActivityDigest | — | in-pr4 |
| `projects.digest.new-commits_other` | {{count}} new commits | projectsActivityDigest | — | in-pr4 |
| `projects.digest.page-title` | Projects Activity | ProjectsOverviewPanel | — | in-pr4 |
| `projects.digest.prefix-currently-tracking` | Currently tracking | projectsActivityDigest | — | in-pr4 |
| `projects.digest.prefix-week` | This week: | projectsActivityDigest | — | in-pr4 |
| `projects.digest.projects_one` | {{count}} project | projectsActivityDigest | — | in-pr4 |
| `projects.digest.projects_other` | {{count}} projects | projectsActivityDigest | — | in-pr4 |
| `projects.digest.reviews-opened_one` | {{count}} review opened | projectsActivityDigest | — | in-pr4 |
| `projects.digest.reviews-opened_other` | {{count}} reviews opened | projectsActivityDigest | — | in-pr4 |
| `projects.digest.reviews_one` | {{count}} review | projectsActivityDigest | — | in-pr4 |
| `projects.digest.reviews_other` | {{count}} reviews | projectsActivityDigest | — | in-pr4 |
| `projects.digest.suffix` | . | projectsActivityDigest | — | in-pr4 |
| `projects.digest.tasks-opened_one` | {{count}} task opened | projectsActivityDigest | — | in-pr4 |
| `projects.digest.tasks-opened_other` | {{count}} tasks opened | projectsActivityDigest | — | in-pr4 |
| `projects.digest.tasks_one` | {{count}} task | projectsActivityDigest | — | in-pr4 |
| `projects.digest.tasks_other` | {{count}} tasks | projectsActivityDigest | — | in-pr4 |
| `projects.home.people` | People | projectHomeWorkspaceSheet | — | in-pr4 |
| `projects.issue.activity` | Activity | ProjectIssuesPanel | — | in-pr4 |
| `projects.issue.assignees` | Assignees | ProjectIssuesPanel | — | in-pr4 |
| `projects.issue.category` | Category | ProjectIssuesPanel | — | in-pr4 |
| `projects.issue.comment-failed` | Failed to post comment. | ProjectIssuesPanel | — | in-pr4 |
| `projects.issue.comment-posted` | Comment posted. | ProjectIssuesPanel | — | in-pr4 |
| `projects.issue.composer-placeholder` | Add a comment… | ProjectIssuesPanel | — | in-pr4 |
| `projects.issue.copy-link` | Copy task link | ProjectIssuesPanel | — | in-pr4 |
| `projects.issue.created` | Task created | ProjectIssuesPanel | — | in-pr4 |
| `projects.issue.created-by` | Created by {{name}} | ProjectIssuesPanel | — | in-pr4 |
| `projects.issue.description` | Description | ProjectIssuesPanel | — | in-pr4 |
| `projects.issue.empty` | No tasks yet | ProjectIssuesPanel | — | in-pr4 |
| `projects.issue.empty-project` | Tasks created for this project's repositories will appear here. | ProjectIssuesPanel | — | in-pr4 |
| `projects.issue.empty-repository` | Tasks created for this repository will appear here. | ProjectIssuesPanel | — | in-pr4 |
| `projects.issue.labels` | Labels | ProjectIssuesPanel | — | in-pr4 |
| `projects.issue.load-error` | Could not load tasks | ProjectIssuesPanel | — | in-pr4 |
| `projects.issue.load-error-hint` | Refresh the project and try again. | ProjectIssuesPanel | — | in-pr4 |
| `projects.issue.loading` | Loading tasks | ProjectIssuesPanel | — | in-pr4 |
| `projects.issue.status` | Status | ProjectIssuesPanel | — | in-pr4 |
| `projects.issue.this-task` | this task | ProjectIssuesPanel | — | in-pr4 |
| `projects.issue.unassigned` | Unassigned | ProjectIssuesPanel | — | in-pr4 |
| `projects.issue.view-comments` | View comments | ProjectIssuesPanel | — | in-pr4 |
| `projects.issue.view-comments-count_one` | View {{count}} comment | ProjectIssuesPanel | — | in-pr4 |
| `projects.issue.view-comments-count_other` | View {{count}} comments | ProjectIssuesPanel | — | in-pr4 |
| `projects.issue.view-task` | View task | ProjectIssuesPanel | — | in-pr4 |
| `projects.overview.action.add-channel` | Add channel | projectsOverviewContext | — | in-pr4 |
| `projects.overview.action.add-repository` | Add repository | projectsOverviewContext | — | in-pr4 |
| `projects.overview.action.create-project` | Create project | projectsOverviewContext | — | in-pr4 |
| `projects.overview.action.create-review` | Create review | ProjectWorkspaceTabs, projectsOverviewContext | — | in-pr4 |
| `projects.overview.action.create-task` | Create task | ProjectWorkspaceTabs, projectsOverviewContext | — | in-pr4 |
| `projects.overview.active` | Active | projectsOverviewContext | — | in-pr4 |
| `projects.overview.active-tasks` | Active tasks | projectsOverviewContext | — | in-pr4 |
| `projects.overview.completed` | Completed | projectsOverviewContext | — | in-pr4 |
| `projects.overview.details` | Details | projectsOverviewContext | — | in-pr4 |
| `projects.overview.merged` | Merged | projectsOverviewContext | — | in-pr4 |
| `projects.overview.open` | Open | projectsOverviewContext | — | in-pr4 |
| `projects.overview.open-reviews` | Open reviews | projectsOverviewContext | — | in-pr4 |
| `projects.overview.repository-activity` | Repository activity | projectsOverviewContext | — | in-pr4 |
| `projects.overview.review-activity` | Review activity | projectsOverviewContext | — | in-pr4 |
| `projects.sections.activity` | Activity | ProjectDetailChrome, ProjectsToolbar, projectsOverviewContext, projectsSectionMeta | — | in-pr4 |
| `projects.sections.channels` | Channels | ProjectsToolbar, projectsOverviewContext, projectsSectionMeta | — | in-pr4 |
| `projects.sections.commits` | Commits | projectHomeWorkspaceSheet | — | in-pr4 |
| `projects.sections.files` | Files | projectHomeWorkspaceSheet | — | in-pr4 |
| `projects.sections.overview` | Overview | ProjectWorkspaceTabList | — | in-pr4 |
| `projects.sections.projects` | Projects | ProjectDetailChrome, ProjectsToolbar, projectsOverviewContext, projectsSectionMeta | — | in-pr4 |
| `projects.sections.repositories` | Repositories | ProjectsToolbar, projectsOverviewContext, projectsSectionMeta | — | in-pr4 |
| `projects.sections.reviews` | Reviews | ProjectWorkspaceTabs, ProjectsToolbar, projectHomeWorkspaceSheet, projectsOverviewContext, projectsSectionMeta | — | in-pr4 |
| `projects.sections.tasks` | Tasks | ProjectsToolbar, projectHomeWorkspaceSheet, projectsOverviewContext, projectsSectionMeta | — | in-pr4 |
| `projects.selection.action.chat-agent` | Chat with an agent | projectSelection | — | in-pr4 |
| `projects.selection.action.copy-link` | Copy link | projectSelection | — | in-pr4 |
| `projects.selection.action.copy-links` | Copy links | projectSelection | — | in-pr4 |
| `projects.selection.action.create-review` | Create review | projectSelection | — | in-pr4 |
| `projects.selection.action.create-reviews` | Create reviews | projectSelection | — | in-pr4 |
| `projects.selection.action.discuss` | Discuss in a channel | projectSelection | — | in-pr4 |
| `projects.selection.count-channel_one` | {{count}} channel | projectSelection | — | in-pr4 |
| `projects.selection.count-channel_other` | {{count}} channels | projectSelection | — | in-pr4 |
| `projects.selection.count-commit_one` | {{count}} commit | projectSelection | — | in-pr4 |
| `projects.selection.count-commit_other` | {{count}} commits | projectSelection | — | in-pr4 |
| `projects.selection.count-project_one` | {{count}} project | projectSelection | — | in-pr4 |
| `projects.selection.count-project_other` | {{count}} projects | projectSelection | — | in-pr4 |
| `projects.selection.count-repository_one` | {{count}} repository | projectSelection | — | in-pr4 |
| `projects.selection.count-repository_other` | {{count}} repositories | projectSelection | — | in-pr4 |
| `projects.selection.count-review_one` | {{count}} review | projectSelection | — | in-pr4 |
| `projects.selection.count-review_other` | {{count}} reviews | projectSelection | — | in-pr4 |
| `projects.selection.count-task_one` | {{count}} task | projectSelection | — | in-pr4 |
| `projects.selection.count-task_other` | {{count}} tasks | projectSelection | — | in-pr4 |
| `projects.sync.pull-local-any` | Pull local commits | projectDetailHelpers | — | in-pr4 |
| `projects.sync.pull-local_one` | Pull {{count}} local commit | projectDetailHelpers | — | in-pr4 |
| `projects.sync.pull-local_other` | Pull {{count}} local commits | projectDetailHelpers | — | in-pr4 |
| `projects.sync.pull-remote-any` | Pull remote commits | projectDetailHelpers | — | in-pr4 |
| `projects.sync.pull-remote_one` | Pull {{count}} remote commit | projectDetailHelpers | — | in-pr4 |
| `projects.sync.pull-remote_other` | Pull {{count}} remote commits | projectDetailHelpers | — | in-pr4 |
| `projects.sync.push-local-any` | Push local commits | projectDetailHelpers | — | in-pr4 |
| `projects.sync.push-local_one` | Push {{count}} local commit | projectDetailHelpers | — | in-pr4 |
| `projects.sync.push-local_other` | Push {{count}} local commits | projectDetailHelpers | — | in-pr4 |
| `projects.sync.push-remote-any` | Push remote commits | projectDetailHelpers | — | in-pr4 |
| `projects.sync.push-remote_one` | Push {{count}} remote commit | projectDetailHelpers | — | in-pr4 |
| `projects.sync.push-remote_other` | Push {{count}} remote commits | projectDetailHelpers | — | in-pr4 |
| `projects.tabs.back` | Back | ProjectWorkspaceTabList | — | in-pr4 |
| `projects.tabs.channels` | Channels | ProjectWorkspaceTabList, ProjectWorkspaceTabs, projectDetailHelpers | — | in-pr4 |
| `projects.tabs.commits` | Commits | ProjectWorkspaceTabList, ProjectWorkspaceTabs, projectDetailHelpers, useProjectDetailCrumbs | — | in-pr4 |
| `projects.tabs.contributors` | Contributors | ProjectWorkspaceTabList, ProjectWorkspaceTabs, projectDetailHelpers | — | in-pr4 |
| `projects.tabs.create-review-hint` | Create review — choose a repository and branches to compare | ProjectWorkspaceTabs | — | in-pr4 |
| `projects.tabs.files` | Files | ProjectWorkspaceTabList, ProjectWorkspaceTabs, projectDetailHelpers | — | in-pr4 |
| `projects.tabs.loading-contributors` | Loading contributors | ProjectWorkspaceTabs | — | in-pr4 |
| `projects.tabs.no-checkout-description` | Switch to the remote source or clone this repository locally. | ProjectWorkspaceTabs | — | in-pr4 |
| `projects.tabs.no-checkout-title` | No local checkout found | ProjectWorkspaceTabs | — | in-pr4 |
| `projects.tabs.not-mirrored` | Not mirrored on Buzz. Repository files are hosted on {{host}}. | ProjectWorkspaceTabs | — | in-pr4 |
| `projects.tabs.publish-update-hint` | Publish the pushed commit to this review | ProjectWorkspaceTabs | — | in-pr4 |
| `projects.tabs.review` | Review | ProjectWorkspaceTabList, projectDetailHelpers, useProjectDetailCrumbs | — | in-pr4 |
| `projects.tabs.tasks` | Tasks | ProjectWorkspaceTabList, ProjectWorkspaceTabs, projectDetailHelpers, useProjectDetailCrumbs | — | in-pr4 |
| `projects.tabs.update-review` | Update review | ProjectWorkspaceTabs | — | in-pr4 |
| `projects.tabs.updating` | Updating… | ProjectWorkspaceTabs | — | in-pr4 |
| `projects.task-category.change-request` | Change request | projectTaskCategories | — | in-pr4 |
| `projects.task-category.improvement` | Improvement | projectTaskCategories | — | in-pr4 |
| `projects.task-category.issue` | Issue | projectTaskCategories | — | in-pr4 |
| `projects.terminal.clone-and-open` | Clone & open in Terminal | useOpenProjectTerminal | — | in-pr4 |
| `projects.terminal.cloned-to` | Cloned to {{path}} | useOpenProjectTerminal | — | in-pr4 |
| `projects.terminal.cloning` | Cloning {{name}}… | useOpenProjectTerminal | — | in-pr4 |
| `projects.terminal.open-failed-description` | Buzz could not open this checkout in your configured terminal. | useOpenProjectTerminal | — | in-pr4 |
| `projects.terminal.open-failed-title` | Couldn’t open terminal | useOpenProjectTerminal | — | in-pr4 |
| `projects.terminal.open-in-terminal` | Open in Terminal | useOpenProjectTerminal | — | in-pr4 |
| `projects.toolbar.grid-layout` | Grid layout | ProjectsToolbar | — | in-pr4 |
| `projects.toolbar.layout-legend` | Project layout | ProjectsToolbar | — | in-pr4 |
| `projects.toolbar.list-layout` | List layout | ProjectsToolbar | — | in-pr4 |
| `projects.toolbar.owner-filter-legend` | Project owner filter | ProjectsToolbar | — | in-pr4 |
| `projects.unavailable.access-description` | Repository access is granted through its channel, and you’re not a member. Ask the repository owner for an invite. | projectRepoAvailability | — | in-pr4 |
| `projects.unavailable.access-title` | Repository access restricted | projectGitError, projectRepoAvailability | — | in-pr4 |
| `projects.unavailable.authentication-description` | Buzz could not authenticate with this repository. Check your access and try again. | projectGitError, projectRepoAvailability | — | in-pr4 |
| `projects.unavailable.authentication-title` | Repository access failed | projectRepoAvailability | — | in-pr4 |
| `projects.unavailable.missing-description` | The project announcement exists, but its git repository was not found on the Buzz relay. | projectRepoAvailability | — | in-pr4 |
| `projects.unavailable.missing-title` | Repository not initialized | projectRepoAvailability | — | in-pr4 |
| `projects.unavailable.network-description` | The Buzz git service could not be reached. Check your connection and try again. | projectRepoAvailability | — | in-pr4 |
| `projects.unavailable.network-title` | Couldn’t reach repository | projectRepoAvailability | — | in-pr4 |
| `projects.unavailable.ref-description` | The selected branch is advertised by the project but is missing from its git remote. | projectRepoAvailability | — | in-pr4 |
| `projects.unavailable.ref-title` | Branch unavailable | projectRepoAvailability | — | in-pr4 |
| `projects.unavailable.unbound-description` | This repository has no access channel binding, so the relay cannot authorize anyone to read it. The repository owner can bind a channel from the Access menu. | projectRepoAvailability | — | in-pr4 |
| `projects.unavailable.unbound-title` | No access channel bound | projectRepoAvailability | — | in-pr4 |
| `projects.unavailable.unknown-description` | Buzz could not load this repository. Try again or contact the project owner. | projectRepoAvailability | — | in-pr4 |
| `projects.unavailable.unknown-title` | Repository unavailable | projectRepoAvailability | — | in-pr4 |

| `agents.access-warning.allowlist-local` | Selected people can use this agent to access your computer, including files, accounts, and connected tools. | agentAccessWarning | — | in-pr4 |
| `agents.access-warning.allowlist-remote` | Selected people can use this agent to access the server it runs on, including any accounts and tools available there. | agentAccessWarning | — | in-pr4 |
| `agents.access-warning.anyone-local` | Anyone can use this agent to access your computer, including files, accounts, and connected tools. | agentAccessWarning | — | in-pr4 |
| `agents.access-warning.anyone-remote` | Anyone can use this agent to access the server it runs on, including any accounts and tools available there. | agentAccessWarning | — | in-pr4 |
| `agents.agent-created.title` | Agent created | useCreatedAgentChannelAttachment | — | in-pr4 |
| `agents.buzz-cli.send-message` | Send Message | agentSessionToolClassifier | — | in-pr4 |
| `agents.buzz-tool.changes-channel` | Changes channel state in the Buzz relay. | agentSessionToolCatalog | — | in-pr4 |
| `agents.buzz-tool.publishes-activity` | Publishes relay-visible Buzz activity. | agentSessionToolCatalog | — | in-pr4 |
| `agents.buzz-tool.reads` | Reads from Buzz. | agentSessionToolCatalog | — | in-pr4 |
| `agents.buzz-tool.reads-channel` | Reads channel context from the Buzz relay. | agentSessionToolCatalog | — | in-pr4 |
| `agents.buzz-tool.reads-identity` | Reads Buzz identity or presence data. | agentSessionToolCatalog | — | in-pr4 |
| `agents.buzz-tool.reads-workflow` | Reads workflow state from Buzz. | agentSessionToolCatalog | — | in-pr4 |
| `agents.buzz-tool.searches-history` | Searches relay-visible Buzz history. | agentSessionToolCatalog | — | in-pr4 |
| `agents.buzz-tool.updates-identity` | Updates Buzz identity or membership data. | agentSessionToolCatalog | — | in-pr4 |
| `agents.buzz-tool.updates-workflow` | Updates workflow state in Buzz. | agentSessionToolCatalog | — | in-pr4 |
| `agents.buzz-tool.writes` | Writes to Buzz. | agentSessionToolCatalog | — | in-pr4 |
| `agents.buzz-verb.added` | Added | agentSessionToolClassifier | — | in-pr4 |
| `agents.buzz-verb.archived` | Archived | agentSessionToolClassifier | — | in-pr4 |
| `agents.buzz-verb.created` | Created | agentSessionToolClassifier | — | in-pr4 |
| `agents.buzz-verb.deleted` | Deleted | agentSessionToolClassifier | — | in-pr4 |
| `agents.buzz-verb.read` | Read | agentSessionToolClassifier | — | in-pr4 |
| `agents.buzz-verb.removed` | Removed | agentSessionToolClassifier | — | in-pr4 |
| `agents.buzz-verb.searched` | Searched | agentSessionToolClassifier | — | in-pr4 |
| `agents.buzz-verb.sent` | Sent | agentSessionToolClassifier | — | in-pr4 |
| `agents.buzz-verb.unarchived` | Unarchived | agentSessionToolClassifier | — | in-pr4 |
| `agents.buzz-verb.updated` | Updated | agentSessionToolClassifier | — | in-pr4 |
| `agents.channel-attach.add-failed` | Failed to add agent. | useCreatedAgentChannelAttachment | — | in-pr4 |
| `agents.channel-attach.added-description` | Added {{agentName}} to #{{channelName}} | useCreatedAgentChannelAttachment | — | in-pr4 |
| `agents.channel-attach.failed-description` | {{agentName}} couldn’t be added to #{{channelName}}. {{error}} | useCreatedAgentChannelAttachment | — | in-pr4 |
| `agents.channel-attach.retrying-description` | Adding {{agentName}} to #{{channelName}}… | useCreatedAgentChannelAttachment | — | in-pr4 |
| `agents.channel-attach.try-again` | Try again | useCreatedAgentChannelAttachment | — | in-pr4 |
| `agents.common.share-to-catalog` | Share to catalog | TeamShareDialog | — | in-pr4 |
| `agents.persona-catalog.add-agent` | Add agent | personaLibraryCopy | — | in-pr4 |
| `agents.persona-catalog.added-action` | Added to My Agents | personaLibraryCopy | — | in-pr4 |
| `agents.persona-catalog.available-state` | Available | personaLibraryCopy | — | in-pr4 |
| `agents.persona-catalog.description` | Browse agents shared to this relay. | personaLibraryCopy | — | in-pr4 |
| `agents.persona-catalog.deselect-action` | Deselect | personaLibraryCopy | — | in-pr4 |
| `agents.persona-catalog.deselect-aria` | Deselect {{displayName}} in My Agents | personaLibraryCopy | — | in-pr4 |
| `agents.persona-catalog.detail-available-description` | Turn this on to make the agent available for teams and agent creation. | personaLibraryCopy | — | in-pr4 |
| `agents.persona-catalog.detail-available-title` | Available in Agent Catalog | personaLibraryCopy | — | in-pr4 |
| `agents.persona-catalog.detail-selected-description` | Turn this off to remove the agent from teams and agent creation in this app. | personaLibraryCopy | — | in-pr4 |
| `agents.persona-catalog.detail-selected-title` | Selected for My Agents | personaLibraryCopy | — | in-pr4 |
| `agents.persona-catalog.details-action` | View details | personaLibraryCopy | — | in-pr4 |
| `agents.persona-catalog.empty-catalog-description` | Shared agents will appear here. | personaLibraryCopy | — | in-pr4 |
| `agents.persona-catalog.empty-catalog-title` | No agents are being shared | personaLibraryCopy | — | in-pr4 |
| `agents.persona-catalog.empty-description` | Everything in Agent Catalog is already in My Agents. | personaLibraryCopy | — | in-pr4 |
| `agents.persona-catalog.empty-title` | You're all set | personaLibraryCopy | — | in-pr4 |
| `agents.persona-catalog.select-action` | Choose | personaLibraryCopy | — | in-pr4 |
| `agents.persona-catalog.select-aria` | Select {{displayName}} in My Agents | personaLibraryCopy | — | in-pr4 |
| `agents.persona-catalog.selected-state` | Selected | personaLibraryCopy | — | in-pr4 |
| `agents.persona-catalog.team-empty-state` | No agents in My Agents yet. Create one or choose one from Agent Catalog first. | personaLibraryCopy | — | in-pr4 |
| `agents.persona-catalog.title` | Agent Catalog | personaLibraryCopy | — | in-pr4 |
| `agents.persona-dialog.copy-name` | {{name}} copy | personaDialogState | — | in-pr4 |
| `agents.persona-dialog.create-agent` | Create agent | personaDialogState | — | in-pr4 |
| `agents.persona-dialog.create-description` | Create an agent and start it immediately. | personaDialogState | — | in-pr4 |
| `agents.persona-dialog.duplicate-description` | Create a new agent by copying this profile and adjusting it as needed. | personaDialogState | — | in-pr4 |
| `agents.persona-dialog.duplicate-title` | Duplicate {{name}} | personaDialogState | — | in-pr4 |
| `agents.persona-dialog.edit-agent` | Edit agent | personaDialogState | — | in-pr4 |
| `agents.persona-dialog.save-changes` | Save changes | personaDialogState | — | in-pr4 |
| `agents.persona-library.choose-from-catalog` | Choose from catalog | personaLibraryCopy | — | in-pr4 |
| `agents.persona-library.create-new` | New agent | personaLibraryCopy | — | in-pr4 |
| `agents.persona-library.description` | The agents you have chosen for this app. Use them to create teams and launch agents. | personaLibraryCopy | — | in-pr4 |
| `agents.persona-library.empty-description` | Choose one from Agent Catalog, create your own, or import one to get started. | personaLibraryCopy | — | in-pr4 |
| `agents.persona-library.empty-import-hint` | Or drop an .agent.json or .agent.png snapshot here to import. | personaLibraryCopy | — | in-pr4 |
| `agents.persona-library.empty-title` | No agents yet | personaLibraryCopy | — | in-pr4 |
| `agents.persona-library.import` | Import snapshot | personaLibraryCopy | — | in-pr4 |
| `agents.persona-library.title` | My agents | personaLibraryCopy | — | in-pr4 |
| `agents.prompt-context.buzz-event` | Buzz event | agentSessionTranscriptHelpers | — | in-pr4 |
| `agents.prompt-context.context` | Context | agentSessionTranscriptHelpers | — | in-pr4 |
| `agents.prompt-context.conversation-context` | Conversation Context | agentSessionTranscriptHelpers | — | in-pr4 |
| `agents.prompt-context.message-window` | {{label}} ({{included}} of {{total}} messages{{truncated}}) | agentSessionTranscriptHelpers | — | in-pr4 |
| `agents.prompt-context.new-messages` | New message — arrived while you were working | agentSessionTranscriptHelpers | — | in-pr4 |
| `agents.prompt-context.new-messages-count_one` | New messages — arrived while you were working — {{count}} events | agentSessionTranscriptHelpers | — | in-pr4 |
| `agents.prompt-context.new-messages-count_other` | New messages — arrived while you were working — {{count}} events | agentSessionTranscriptHelpers | — | in-pr4 |
| `agents.prompt-context.new-request` | New request — supersedes previous | agentSessionTranscriptHelpers | — | in-pr4 |
| `agents.prompt-context.new-request-count_one` | New request — supersedes previous — {{count}} events | agentSessionTranscriptHelpers | — | in-pr4 |
| `agents.prompt-context.new-request-count_other` | New request — supersedes previous — {{count}} events | agentSessionTranscriptHelpers | — | in-pr4 |
| `agents.prompt-context.previous-request` | Previous request — interrupted before completion | agentSessionTranscriptHelpers | — | in-pr4 |
| `agents.prompt-context.thread-context` | Thread Context | agentSessionTranscriptHelpers | — | in-pr4 |
| `agents.prompt-context.truncated-suffix` | , truncated | agentSessionTranscriptHelpers | — | in-pr4 |
| `agents.prompt-context.what-you-were-working-on` | What you were working on | agentSessionTranscriptHelpers | — | in-pr4 |
| `agents.prompt-section.agent-instructions` | Agent Instructions | agentSessionTranscriptHelpers | — | in-pr4 |
| `agents.prompt-section.base` | Base | agentSessionTranscriptHelpers | — | in-pr4 |
| `agents.prompt-section.channel-canvas` | Channel Canvas | agentSessionTranscriptHelpers | — | in-pr4 |
| `agents.prompt-section.core-memory` | Core Memory | agentSessionTranscriptHelpers | — | in-pr4 |
| `agents.prompt-section.huddle-instructions` | Huddle Instructions | agentSessionTranscriptHelpers | — | in-pr4 |
| `agents.prompt-section.prompt` | Prompt | agentSessionTranscriptHelpers | — | in-pr4 |
| `agents.prompt-section.system` | System | agentSessionTranscriptHelpers | — | in-pr4 |
| `agents.prompt-section.team-instructions` | Team Instructions | TeamDialog, agentSessionTranscriptHelpers | — | in-pr4 |
| `agents.prompt-section.workspace` | Workspace | agentSessionTranscriptHelpers | — | in-pr4 |
| `agents.remove-members.cancel` | Cancel | RemoveMembersConfirmDialog | — | in-pr4 |
| `agents.remove-members.description_one` | {{memberNames}} will be removed from this team. Do you also want to remove the agent completely? | RemoveMembersConfirmDialog | — | in-pr4 |
| `agents.remove-members.description_other` | {{memberNames}} will be removed from this team. Do you also want to remove the agents completely? | RemoveMembersConfirmDialog | — | in-pr4 |
| `agents.remove-members.keep-agent_one` | Keep agent | RemoveMembersConfirmDialog | — | in-pr4 |
| `agents.remove-members.keep-agent_other` | Keep agents | RemoveMembersConfirmDialog | — | in-pr4 |
| `agents.remove-members.remove-agent_one` | Remove agent | RemoveMembersConfirmDialog | — | in-pr4 |
| `agents.remove-members.remove-agent_other` | Remove agents | RemoveMembersConfirmDialog | — | in-pr4 |
| `agents.remove-members.title_one` | Remove {{count}} member? | RemoveMembersConfirmDialog | — | in-pr4 |
| `agents.remove-members.title_other` | Remove {{count}} members? | RemoveMembersConfirmDialog | — | in-pr4 |
| `agents.team-actions.add-failed` | Failed to add team. | useTeamActions | — | in-pr4 |
| `agents.team-actions.added` | Added {{name}} to your teams. | useTeamActions | — | in-pr4 |
| `agents.team-actions.already-added` | {{name}} is already in your teams. | useTeamActions | — | in-pr4 |
| `agents.team-actions.copy-name` | {{name}} copy | useTeamActions | — | in-pr4 |
| `agents.team-actions.create-description` | Group agents together for quick deployment to channels. | useTeamActions | — | in-pr4 |
| `agents.team-actions.create-team` | Create team | useTeamActions | — | in-pr4 |
| `agents.team-actions.created` | Created team "{{name}}". | useTeamActions | — | in-pr4 |
| `agents.team-actions.delete-failed` | Failed to delete team. | useTeamActions | — | in-pr4 |
| `agents.team-actions.deleted` | Deleted team "{{name}}". | useTeamActions | — | in-pr4 |
| `agents.team-actions.deployed-partial_one` | Deployed {{count}} agent to {{channelName}}. {{failed}} failed. | useTeamActions | — | in-pr4 |
| `agents.team-actions.deployed-partial_other` | Deployed {{count}} agents to {{channelName}}. {{failed}} failed. | useTeamActions | — | in-pr4 |
| `agents.team-actions.deployed_one` | Deployed {{count}} agent to {{channelName}}. | useTeamActions | — | in-pr4 |
| `agents.team-actions.deployed_other` | Deployed {{count}} agents to {{channelName}}. | useTeamActions | — | in-pr4 |
| `agents.team-actions.duplicate-description` | Create a new team by copying this one. | useTeamActions | — | in-pr4 |
| `agents.team-actions.duplicate-title` | Duplicate {{name}} | useTeamActions | — | in-pr4 |
| `agents.team-actions.edit-team` | Edit team | useTeamActions | — | in-pr4 |
| `agents.team-actions.export-snapshot-failed` | Failed to export team snapshot. | useTeamActions | — | in-pr4 |
| `agents.team-actions.exported` | Exported {{name}}. | useTeamActions | — | in-pr4 |
| `agents.team-actions.import-snapshot-failed` | Failed to import team snapshot. | useTeamActions | — | in-pr4 |
| `agents.team-actions.read-snapshot-failed` | Failed to read team snapshot file. | useTeamActions | — | in-pr4 |
| `agents.team-actions.save-changes` | Save changes | useTeamActions | — | in-pr4 |
| `agents.team-actions.save-failed` | Failed to save team. | useTeamActions | — | in-pr4 |
| `agents.team-actions.share-failed` | Failed to share team. | useTeamActions | — | in-pr4 |
| `agents.team-actions.unshare-failed` | Failed to unshare team. | useTeamActions | — | in-pr4 |
| `agents.team-actions.updated` | Updated team "{{name}}". | useTeamActions | — | in-pr4 |
| `agents.team-catalog.add-team` | Add team | teamLibraryCopy | — | in-pr4 |
| `agents.team-catalog.added-team` | Added to my teams | teamLibraryCopy | — | in-pr4 |
| `agents.team-catalog.adding-team` | Adding… | teamLibraryCopy | — | in-pr4 |
| `agents.team-catalog.auto-retracted` | "{{teamName}}" has been queued for removal from the community catalog because it can no longer be projected: {{reason}} | teamLibraryCopy | — | in-pr4 |
| `agents.team-catalog.choose-from-catalog` | Choose from catalog | teamLibraryCopy | — | in-pr4 |
| `agents.team-catalog.description` | Browse teams shared to this relay. | teamLibraryCopy | — | in-pr4 |
| `agents.team-catalog.empty-description` | Shared teams will appear here. | teamLibraryCopy | — | in-pr4 |
| `agents.team-catalog.empty-title` | No teams are being shared | teamLibraryCopy | — | in-pr4 |
| `agents.team-catalog.published` | Published {{teamName}} to the community catalog. | teamLibraryCopy | — | in-pr4 |
| `agents.team-catalog.share-description` | Anyone in this community can find and add a copy of this team. Both the team instructions and every member’s instructions are shared as plaintext. Memories and secrets aren’t included. | teamLibraryCopy | — | in-pr4 |
| `agents.team-catalog.share-queued` | Sharing {{teamName}} is queued. It will appear after the relay accepts the update. | teamLibraryCopy | — | in-pr4 |
| `agents.team-catalog.share-title` | Share to catalog | teamLibraryCopy | — | in-pr4 |
| `agents.team-catalog.title` | Team Catalog | teamLibraryCopy | — | in-pr4 |
| `agents.team-catalog.unshare-queued` | Removing {{teamName}} is queued. It may remain discoverable until the relay accepts the update. | teamLibraryCopy | — | in-pr4 |
| `agents.team-catalog.unshared` | {{teamName}} is no longer discoverable in the community catalog. | teamLibraryCopy | — | in-pr4 |
| `agents.team-dialog.agents` | Agents | TeamDialog | — | in-pr4 |
| `agents.team-dialog.built-in` | Built-in | TeamDialog | — | in-pr4 |
| `agents.team-dialog.cancel` | Cancel | TeamDialog | — | in-pr4 |
| `agents.team-dialog.description` | Description | TeamDialog | — | in-pr4 |
| `agents.team-dialog.description-placeholder` | Optional description for this team. | TeamDialog | — | in-pr4 |
| `agents.team-dialog.instructions-placeholder` | Optional instructions applied to every deployed team member. | TeamDialog | — | in-pr4 |
| `agents.team-dialog.missing-personas_one` | This team references {{count}} agent that is no longer in My Agents. Save to remove them, or add them back to My Agents first. | TeamDialog | — | in-pr4 |
| `agents.team-dialog.missing-personas_other` | This team references {{count}} agents that are no longer in My Agents. Save to remove them, or add them back to My Agents first. | TeamDialog | — | in-pr4 |
| `agents.team-dialog.name` | Name | TeamDialog | — | in-pr4 |
| `agents.team-dialog.saving` | Saving... | TeamDialog | — | in-pr4 |
| `agents.team-dialog.select-agents` | Select the agents to include in this team. | TeamDialog | — | in-pr4 |
| `agents.team-import.and-separator` |  and  | teamSnapshotImport.lib | — | in-pr4 |
| `agents.team-import.memory-errors_one` | {{count}} memory entry failed to restore | teamSnapshotImport.lib | — | in-pr4 |
| `agents.team-import.memory-errors_other` | {{count}} memory entries failed to restore | teamSnapshotImport.lib | — | in-pr4 |
| `agents.team-import.partial` | {{name}} imported, but {{parts}}. | teamSnapshotImport.lib | — | in-pr4 |
| `agents.team-import.profile-sync-errors_one` | {{count}} member failed to sync profile | teamSnapshotImport.lib | — | in-pr4 |
| `agents.team-import.profile-sync-errors_other` | {{count}} members failed to sync profiles | teamSnapshotImport.lib | — | in-pr4 |
| `agents.team-import.success_one` | Imported {{name}} with {{count}} member. | teamSnapshotImport.lib | — | in-pr4 |
| `agents.team-import.success_other` | Imported {{name}} with {{count}} members. | teamSnapshotImport.lib | — | in-pr4 |
| `agents.teams-section.actions-aria` | {{teamName}} team actions | TeamsSection | — | in-pr4 |
| `agents.teams-section.create-team` | Create team | TeamsSection | — | in-pr4 |
| `agents.teams-section.delete` | Delete | TeamsSection | — | in-pr4 |
| `agents.teams-section.deploy-to-channel` | Deploy to channel | TeamsSection | — | in-pr4 |
| `agents.teams-section.description` | Group agents that you can add to a channel together. | TeamsSection | — | in-pr4 |
| `agents.teams-section.duplicate` | Duplicate | TeamsSection | — | in-pr4 |
| `agents.teams-section.edit` | Edit | TeamsSection | — | in-pr4 |
| `agents.teams-section.import` | Import | TeamsSection | — | in-pr4 |
| `agents.teams-section.new-team` | New team | TeamsSection | — | in-pr4 |
| `agents.teams-section.share` | Share | TeamsSection | — | in-pr4 |
| `agents.teams-section.title` | Agent teams | TeamsSection | — | in-pr4 |
| `agents.tool-class.error` | Error | agentSessionToolClassifier | — | in-pr4 |
| `agents.tool-class.file-edit` | File edit | agentSessionToolClassifier | — | in-pr4 |
| `agents.tool-class.file-read` | File read | agentSessionToolClassifier | — | in-pr4 |
| `agents.tool-class.generic` | Tool | agentSessionToolClassifier | — | in-pr4 |
| `agents.tool-class.image` | Image | agentSessionToolClassifier | — | in-pr4 |
| `agents.tool-class.message` | Message | agentSessionToolClassifier | — | in-pr4 |
| `agents.tool-class.permission` | Permission | agentSessionToolClassifier | — | in-pr4 |
| `agents.tool-class.plan` | Plan | agentSessionToolClassifier | — | in-pr4 |
| `agents.tool-class.raw-rail` | Raw event | agentSessionToolClassifier | — | in-pr4 |
| `agents.tool-class.relay-op` | Buzz relay op | agentSessionToolClassifier | — | in-pr4 |
| `agents.tool-class.shell` | Shell command | agentSessionToolClassifier | — | in-pr4 |
| `agents.tool-class.skill-read` | Skill read | agentSessionToolClassifier | — | in-pr4 |
| `agents.tool-class.status` | Status | agentSessionToolClassifier | — | in-pr4 |
| `agents.tool-class.suppressed` | Suppressed | agentSessionToolClassifier | — | in-pr4 |
| `agents.tool-class.thought` | Thought | agentSessionToolClassifier | — | in-pr4 |
| `agents.tool-label.checked-todos` | Checked todos | agentSessionToolClassifier | — | in-pr4 |
| `agents.tool-label.context-compacted` | Context compacted | agentSessionToolClassifier | — | in-pr4 |
| `agents.tool-label.edited-file` | Edited file | agentSessionToolClassifier | — | in-pr4 |
| `agents.tool-label.failed-suffix` | {{label}} failed | agentSessionToolClassifier | — | in-pr4 |
| `agents.tool-label.ran-command` | Ran command | agentSessionToolClassifier | — | in-pr4 |
| `agents.tool-label.ran-tool` | Ran tool | agentSessionToolClassifier | — | in-pr4 |
| `agents.tool-label.read-file` | Read file | agentSessionToolClassifier | — | in-pr4 |
| `agents.tool-label.read-skill` | Read skill | agentSessionToolClassifier | — | in-pr4 |
| `agents.tool-label.read-skill-file` | Read skill file | agentSessionToolClassifier | — | in-pr4 |
| `agents.tool-label.updated-todos` | Updated todos | agentSessionToolClassifier | — | in-pr4 |
| `agents.tool-label.viewed-image` | Viewed image | agentSessionToolClassifier | — | in-pr4 |
| `agents.tool-object.command` | command | agentSessionToolClassifier | — | in-pr4 |
| `agents.tool-object.context` | context | agentSessionToolClassifier | — | in-pr4 |
| `agents.tool-object.file` | file | agentSessionToolClassifier | — | in-pr4 |
| `agents.tool-object.image` | image | agentSessionToolClassifier | — | in-pr4 |
| `agents.tool-object.message` | message | agentSessionToolClassifier | — | in-pr4 |
| `agents.tool-object.skill` | skill | agentSessionToolClassifier | — | in-pr4 |
| `agents.tool-object.todos` | todos | agentSessionToolClassifier | — | in-pr4 |
| `agents.tool-object.tool` | tool | agentSessionToolClassifier | — | in-pr4 |
| `agents.tool-status.done` | Done | agentSessionToolCatalog | — | in-pr4 |
| `agents.tool-status.error` | Error | agentSessionToolCatalog | — | in-pr4 |
| `agents.tool-status.pending` | Pending | agentSessionToolCatalog | — | in-pr4 |
| `agents.tool-status.running` | Running | agentSessionToolCatalog | — | in-pr4 |
| `agents.tool-verb.checked` | Checked | agentSessionToolClassifier | — | in-pr4 |
| `agents.tool-verb.compacted` | Compacted | agentSessionToolClassifier | — | in-pr4 |
| `agents.tool-verb.edited` | Edited | agentSessionToolClassifier | — | in-pr4 |
| `agents.tool-verb.ran` | Ran | agentSessionToolClassifier | — | in-pr4 |
| `agents.tool-verb.read` | Read | agentSessionToolClassifier | — | in-pr4 |
| `agents.tool-verb.updated` | Updated | agentSessionToolClassifier | — | in-pr4 |
| `agents.tool-verb.viewed` | Viewed | agentSessionToolClassifier | — | in-pr4 |
| `agents.transcript.new-session-created` | New session created. | agentSessionTranscriptHelpers | — | in-pr4 |
| `agents.transcript.tool-call` | Tool call | agentSessionTranscriptHelpers | — | in-pr4 |

| `notifications.permission.blocked` | Desktop notifications are blocked for Buzz. Enable them in system settings to turn alerts on. | hooks | — | in-pr4 |
| `notifications.permission.enable-failed` | Failed to enable desktop notifications. | hooks | — | in-pr4 |
| `notifications.permission.unavailable` | Desktop notifications are unavailable in this environment. | hooks | — | in-pr4 |
| `notifications.sound.desc-agent-job-accepted` | When an agent picks up a job. | sound | — | in-pr4 |
| `notifications.sound.desc-agent-job-error` | When an agent job fails. | sound | — | in-pr4 |
| `notifications.sound.desc-agent-job-progress` | While an agent works through a job. | sound | — | in-pr4 |
| `notifications.sound.desc-agent-job-result` | When an agent finishes a job. | sound | — | in-pr4 |
| `notifications.sound.desc-direct-messages` | When someone messages you directly. | sound | — | in-pr4 |
| `notifications.sound.desc-mentions` | When someone tags you in a channel. | sound | — | in-pr4 |
| `notifications.sound.desc-needs-action` | When an approval or reminder is waiting on you. | sound | — | in-pr4 |
| `notifications.sound.desc-thread-replies` | When someone replies in a thread you follow or posted in. | sound | — | in-pr4 |
| `notifications.sound.slot-agent-job-accepted` | Agent: job accepted | sound | — | in-pr4 |
| `notifications.sound.slot-agent-job-error` | Agent: job error | sound | — | in-pr4 |
| `notifications.sound.slot-agent-job-progress` | Agent: progress update | sound | — | in-pr4 |
| `notifications.sound.slot-agent-job-result` | Agent: job result | sound | — | in-pr4 |
| `notifications.sound.slot-mentions` | @Mentions | sound | — | in-pr4 |
| `notifications.sound.slot-needs-action` | Needs action | sound | — | in-pr4 |
| `notifications.sound.slot-thread-replies` | Thread replies | sound | — | in-pr4 |
| `notifications.toast.body-approval` | A workflow is waiting for your approval. | notificationFormat | — | in-pr4 |
| `notifications.toast.body-needs-attention` | Something in Buzz needs your attention. | notificationFormat | — | in-pr4 |
| `notifications.toast.body-new-message` | New message | notificationFormat | — | in-pr4 |
| `notifications.toast.body-new-reply` | New reply | notificationFormat | — | in-pr4 |
| `notifications.toast.body-truncation-marker` | ... | notificationFormat | — | in-pr4 |
| `notifications.toast.title-approval` | Approval Requested | notificationFormat | — | in-pr4 |
| `notifications.toast.title-approval-sender` | {{senderName}} requested approval | notificationFormat | — | in-pr4 |
| `notifications.toast.title-direct-message` | Direct message | notificationFormat | — | in-pr4 |
| `notifications.toast.title-in-channel` | {{prefix}} in {{channelLabel}} | notificationFormat | — | in-pr4 |
| `notifications.toast.title-mention` | @Mention | notificationFormat | — | in-pr4 |
| `notifications.toast.title-mention-sender` | {{senderName}} mentioned you | notificationFormat | — | in-pr4 |
| `notifications.toast.title-needs-action` | Needs Action | notificationFormat | — | in-pr4 |
| `notifications.toast.title-reply` | Reply | notificationFormat | — | in-pr4 |
| `notifications.toast.title-reply-sender` | {{senderName}} replied | notificationFormat | — | in-pr4 |

| `search.context.direct-message` | Direct message | TopbarSearch | — | in-pr4 |
| `search.context.message` | Message | SearchResultItem, TopbarSearch | — | in-pr4 |
| `search.context.message-in` | Message in | TopbarSearch | — | in-pr4 |
| `search.context.thread-in` | Thread in | TopbarSearch | — | in-pr4 |
| `search.empty.in-scope` | in | TopbarSearch | — | in-pr4 |
| `search.empty.no-matches-for` | No matches for | TopbarSearch | — | in-pr4 |
| `search.empty.no-messages-for` | No messages for | TopbarSearch | — | in-pr4 |
| `search.empty.terminal` | . | TopbarSearch | — | in-pr4 |
| `search.kind.agent-job` | Agent job | SearchResultItem | — | in-pr4 |
| `search.kind.agent-update` | Agent update | SearchResultItem | — | in-pr4 |
| `search.kind.approval-request` | Approval request | SearchResultItem | — | in-pr4 |
| `search.kind.channel` | Channel | SearchResultItem | — | in-pr4 |
| `search.kind.forum-post` | Forum post | SearchResultItem | — | in-pr4 |
| `search.kind.forum-reply` | Forum reply | SearchResultItem | — | in-pr4 |
| `search.kind.note` | Note | SearchResultItem | — | in-pr4 |
| `search.preview.no-message-body` | No message body. | SearchResultItem, searchMatch | — | in-pr4 |
| `search.prompt.a-channel` | a channel | SearchPromptPlaceholder | — | in-pr4 |
| `search.prompt.a-message` | a message | SearchPromptPlaceholder | — | in-pr4 |
| `search.prompt.a-thread` | a thread | SearchPromptPlaceholder | — | in-pr4 |
| `search.prompt.an-agent` | an agent | SearchPromptPlaceholder | — | in-pr4 |
| `search.prompt.everything` | everything | SearchPromptPlaceholder | — | in-pr4 |
| `search.prompt.search-for` | Search for | SearchPromptPlaceholder | — | in-pr4 |
| `search.relative.days-ago` | {{count}}d ago | TopbarSearch | — | in-pr4 |
| `search.relative.hours-ago` | {{count}}h ago | SearchResultItem, TopbarSearch | — | in-pr4 |
| `search.relative.minutes-ago` | {{count}}m ago | SearchResultItem, TopbarSearch | — | in-pr4 |
| `search.results.actions` | Actions | TopbarSearch | — | in-pr4 |
| `search.results.most-relevant` | Most relevant | TopbarSearch | — | in-pr4 |
| `search.results.no-recent-activity` | No recent activity yet. | TopbarSearch | — | in-pr4 |
| `search.results.people` | People | TopbarSearch | — | in-pr4 |
| `search.results.recent-activity` | Recent activity | TopbarSearch | — | in-pr4 |
| `search.scope.action-conversation-with` | Search conversation with | SearchScopeControls | — | in-pr4 |
| `search.scope.action-in` | Search in | SearchScopeControls | — | in-pr4 |
| `search.scope.hint-channel` | Search messages in this channel | SearchScopeControls | — | in-pr4 |
| `search.scope.hint-conversation` | Search messages in this conversation. | SearchScopeControls | — | in-pr4 |
| `search.scope.remove-scope` | Remove {{scopeLabel}} search scope | SearchScopeControls | — | in-pr4 |
| `search.scope.search-in` | Search in {{scopeLabel}} | SearchScopeControls, TopbarSearch | — | in-pr4 |
| `search.scope.search-messages` | Search messages | SearchScopeControls | — | in-pr4 |
| `search.trigger.search-everything` | Search everything | SearchScopeControls, TopbarSearch | — | in-pr4 |

| `home.feed.channel-fallback` | channel | FeedSection | — | in-pr4 |
| `home.feed.mark-done` | Mark done | FeedSection | — | in-pr4 |
| `home.feed.no-response` | The relay did not return a feed response. | HomeView | — | in-pr4 |
| `home.feed.unavailable` | Home feed unavailable | HomeView | — | in-pr4 |
| `home.feed.undo-done` | Undo done | FeedSection | — | in-pr4 |
| `home.filter.active-drafts_one` | {{count}} active draft | InboxFilterMenu | — | in-pr4 |
| `home.filter.active-drafts_other` | {{count}} active drafts | InboxFilterMenu | — | in-pr4 |
| `home.filter.all` | All | InboxFilterMenu | — | in-pr4 |
| `home.filter.due-reminders_one` | {{count}} due reminder | InboxFilterMenu | — | in-pr4 |
| `home.filter.due-reminders_other` | {{count}} due reminders | InboxFilterMenu | — | in-pr4 |
| `home.filter.mentions` | Mentions | InboxFilterMenu | — | in-pr4 |
| `home.filter.needs-action` | Needs action | InboxFilterMenu | — | in-pr4 |
| `home.filter.reminders` | Reminders | InboxFilterMenu | — | in-pr4 |
| `home.filter.threads` | Threads | InboxFilterMenu | — | in-pr4 |
| `home.filter.trigger-aria` | Filter inbox: {{filter}} | InboxFilterMenu | — | in-pr4 |
| `home.filter.trigger-aria-status` | Filter inbox: {{filter}}. {{status}} | InboxFilterMenu | — | in-pr4 |
| `home.inbox.back-to-list` | Back to inbox list | InboxDetailPane | — | in-pr4 |
| `home.inbox.category-activity` | Activity | inbox | — | in-pr4 |
| `home.inbox.category-agent-update` | Agent update | inbox | — | in-pr4 |
| `home.inbox.category-mention` | Mention | inbox | — | in-pr4 |
| `home.inbox.category-needs-action` | Needs Action | inbox | — | in-pr4 |
| `home.inbox.composer-dm` | Message {{name}} | InboxDetailPane | — | in-pr4 |
| `home.inbox.composer-reply-thread` | Send reply to #{{channel}} thread | InboxDetailPane | — | in-pr4 |
| `home.inbox.composer-reply-thread-generic` | Send reply to channel thread | InboxDetailPane | — | in-pr4 |
| `home.inbox.context-error` | Some message context could not be loaded. | InboxDetailPane | — | in-pr4 |
| `home.inbox.couldnt-reopen` | Couldn’t reopen | InboxDetailPane, InboxListPane | — | in-pr4 |
| `home.inbox.detail-empty-hint` | Pick an inbox item to see the full message and react to it. | InboxDetailPane | — | in-pr4 |
| `home.inbox.detail-empty-title` | Select a message | InboxDetailPane | — | in-pr4 |
| `home.inbox.dm-with` | DM with {{name}} | InboxDetailPane | — | in-pr4 |
| `home.inbox.empty-agent-activity` | No agent updates found | InboxListPane | — | in-pr4 |
| `home.inbox.empty-all` | No activity yet | InboxListPane | — | in-pr4 |
| `home.inbox.empty-all-hint` | New activity will appear here. | InboxListPane | — | in-pr4 |
| `home.inbox.empty-filter-hint` | Switch back to All to see other activity. | InboxListPane | — | in-pr4 |
| `home.inbox.empty-mention` | No mentions found | InboxListPane | — | in-pr4 |
| `home.inbox.empty-needs-action` | Nothing needs action | InboxListPane | — | in-pr4 |
| `home.inbox.empty-project` | No project work found | InboxListPane | — | in-pr4 |
| `home.inbox.empty-reminders` | No reminders | InboxListPane | — | in-pr4 |
| `home.inbox.empty-thread` | No threads found | InboxListPane | — | in-pr4 |
| `home.inbox.headline-agent-update` | Agent update | FeedSection, inbox | — | in-pr4 |
| `home.inbox.headline-approval-requested` | Approval requested | FeedSection, inbox | — | in-pr4 |
| `home.inbox.headline-channel-update` | Channel update | FeedSection, inbox | — | in-pr4 |
| `home.inbox.headline-forum-post` | Forum post | FeedSection, inbox | — | in-pr4 |
| `home.inbox.headline-forum-reply` | Forum reply | FeedSection, inbox | — | in-pr4 |
| `home.inbox.headline-job-accepted` | Job accepted | FeedSection, inbox | — | in-pr4 |
| `home.inbox.headline-job-cancelled` | Job cancelled | FeedSection, inbox | — | in-pr4 |
| `home.inbox.headline-job-failed` | Job failed | FeedSection, inbox | — | in-pr4 |
| `home.inbox.headline-job-requested` | Job requested | FeedSection, inbox | — | in-pr4 |
| `home.inbox.headline-job-result` | Job result | FeedSection, inbox | — | in-pr4 |
| `home.inbox.headline-mention` | Mention | FeedSection, inbox | — | in-pr4 |
| `home.inbox.headline-progress-update` | Progress update | FeedSection, inbox | — | in-pr4 |
| `home.inbox.headline-project-update` | Project update | inbox | — | in-pr4 |
| `home.inbox.headline-reminder` | Reminder | FeedSection, inbox | — | in-pr4 |
| `home.inbox.headline-review` | Review | inbox | — | in-pr4 |
| `home.inbox.headline-task` | Task | inbox | — | in-pr4 |
| `home.inbox.in` | In | InboxListPane | — | in-pr4 |
| `home.inbox.in-dm-with` | In DM with {{name}} | InboxListPane | — | in-pr4 |
| `home.inbox.loading-context` | Loading surrounding context... | InboxDetailPane | — | in-pr4 |
| `home.inbox.message-in` | Message in #{{channel}} | InboxDetailPane | — | in-pr4 |
| `home.inbox.no-channel-link` | No channel link | InboxListPane | — | in-pr4 |
| `home.inbox.open-conversation` | Open conversation | InboxDetailPane | — | in-pr4 |
| `home.inbox.open-full-thread` | Open full thread | InboxDetailPane | — | in-pr4 |
| `home.inbox.open-in-channel` | Open in channel | InboxDetailPane, InboxListPane | — | in-pr4 |
| `home.inbox.open-item-aria` | Open inbox item from {{name}} | InboxListPane | — | in-pr4 |
| `home.inbox.options-aria` | Inbox options | InboxListPane | — | in-pr4 |
| `home.inbox.preview-approval-waiting` | A workflow is waiting for approval. | FeedSection, inbox | — | in-pr4 |
| `home.inbox.preview-no-details` | No additional details were attached to this event. | FeedSection, inbox | — | in-pr4 |
| `home.inbox.preview-reminder-waiting` | A reminder is waiting for you. | FeedSection, inbox | — | in-pr4 |
| `home.inbox.remind-needs-channel` | Cannot remind without a channel | InboxListPane | — | in-pr4 |
| `home.inbox.reminder-set` | Reminder set | InboxListPane | — | in-pr4 |
| `home.inbox.reopen-failed-toast` | Could not reopen conversation. Try again. | useHiddenDmInboxNavigation | — | in-pr4 |
| `home.inbox.reopening` | Reopening… | InboxDetailPane, InboxListPane | — | in-pr4 |
| `home.inbox.replies-unavailable` | Replies are not available for this item. | HomeView, InboxDetailPane | — | in-pr4 |
| `home.inbox.reply-no-inline` | This item does not support inline replies yet. | homeMessageCapabilities | — | in-pr4 |
| `home.inbox.reply-no-target` | This inbox item does not have a reply target. | homeMessageCapabilities | — | in-pr4 |
| `home.inbox.reply-open-linked` | Open the linked channel to reply. | homeMessageCapabilities | — | in-pr4 |
| `home.inbox.show-unread-only` | Show unread only | InboxListPane | — | in-pr4 |
| `home.inbox.thread-in` | Thread in #{{channel}} | InboxDetailPane | — | in-pr4 |
| `home.inbox.thread-with` | Thread with {{name}} | InboxDetailPane | — | in-pr4 |
| `home.inbox.type-dm` | DM | inbox | — | in-pr4 |
| `home.inbox.type-dm-from` | DM from {{name}} | inbox | — | in-pr4 |
| `home.inbox.type-headline-in` | {{headline}} in | inbox | — | in-pr4 |
| `home.inbox.type-mentioned` | Mentioned | inbox | — | in-pr4 |
| `home.inbox.type-mentioned-in` | Mentioned in | inbox | — | in-pr4 |
| `home.inbox.type-needs-action` | Needs action | inbox | — | in-pr4 |
| `home.inbox.type-needs-action-in` | Needs action in | inbox | — | in-pr4 |
| `home.inbox.type-thread-in` | Thread in | inbox | — | in-pr4 |
| `home.inbox.unread-count` | {{count}} unread | InboxListPane | — | in-pr4 |
| `home.inbox.unread-empty-agent-activity` | No unread agent updates | InboxListPane | — | in-pr4 |
| `home.inbox.unread-empty-all` | No unread activity | InboxListPane | — | in-pr4 |
| `home.inbox.unread-empty-drafts` | No unread drafts | InboxListPane | — | in-pr4 |
| `home.inbox.unread-empty-hint` | Turn off Show unread only to see read activity. | InboxListPane | — | in-pr4 |
| `home.inbox.unread-empty-mention` | No unread mentions | InboxListPane | — | in-pr4 |
| `home.inbox.unread-empty-needs-action` | No unread items needing action | InboxListPane | — | in-pr4 |
| `home.inbox.unread-empty-project` | No unread project work | InboxListPane | — | in-pr4 |
| `home.inbox.unread-empty-reminders` | No unread reminders | InboxListPane | — | in-pr4 |
| `home.inbox.unread-empty-thread` | No unread threads | InboxListPane | — | in-pr4 |
| `home.notes.days-ago` | {{count}}d | RecentNotesSection | — | in-pr4 |
| `home.notes.heading` | Recent Notes | RecentNotesSection | — | in-pr4 |
| `home.notes.hours-ago` | {{count}}h | RecentNotesSection | — | in-pr4 |
| `home.notes.minutes-ago` | {{count}}m | RecentNotesSection | — | in-pr4 |
| `home.notes.view-all-pulse` | View all in Pulse | RecentNotesSection | — | in-pr4 |
| `home.project.activity-partial` | Some project activity could not be loaded. Actions are unavailable until the item is current. | ProjectInboxDetail | — | in-pr4 |
| `home.project.back-to-inbox` | Back to Inbox | ProjectInboxDetail, ProjectInboxDetailPane | — | in-pr4 |
| `home.project.load-failed` | Could not load this project item. | ProjectInboxDetail | — | in-pr4 |
| `home.project.loading` | Loading project item… | ProjectInboxDetail | — | in-pr4 |
| `home.project.merge-recovery-reviews-only` | Merge recovery is only available for reviews. | ProjectInboxDetailPane | — | in-pr4 |
| `home.project.no-clone-url` | This repository has no clone URL. | ProjectInboxDetailPane | — | in-pr4 |
| `home.project.not-found` | This project item could not be found. | ProjectInboxDetail | — | in-pr4 |
| `home.project.open-review` | Open review | ProjectInboxDetailPane | — | in-pr4 |
| `home.project.open-task` | Open task | ProjectInboxDetailPane | — | in-pr4 |
| `home.project.sent-review` | {{name}} sent you a review | ProjectInboxDetailPane | — | in-pr4 |
| `home.project.sent-task` | {{name}} sent you a task | ProjectInboxDetailPane | — | in-pr4 |
| `home.reminder.due` | Reminder due | InboxListPane | — | in-pr4 |
| `home.reminder.in-days` | Reminder in {{count}}d | InboxListPane | — | in-pr4 |
| `home.reminder.in-hours` | Reminder in {{count}}h | InboxListPane | — | in-pr4 |
| `home.reminder.in-minutes` | Reminder in {{count}}m | InboxListPane | — | in-pr4 |
| `home.reminder.in-under-minute` | Reminder in less than a minute | InboxListPane | — | in-pr4 |
| `home.reminder.label` | Reminder | InboxListPane | — | in-pr4 |
| `home.reminder.pending` | Pending | InboxListPane | — | in-pr4 |
| `home.resizer.aria` | Resize inbox list | HomeView | — | in-pr4 |
| `home.resizer.drag` | Drag to resize. | HomeView | — | in-pr4 |
| `home.resizer.drag-reset` | Drag to resize. Double-click to reset width. | HomeView | — | in-pr4 |

| `common.date.day-and-time` | {{date}} at {{time}} | common surface | — | in-pr4 |
| `common.date.today` | Today | common surface | — | in-pr4 |
| `common.date.yesterday` | Yesterday | common surface | — | in-pr4 |

| `workflows.action-type.add-reaction` | Add Reaction | workflowFormTypes | — | in-pr4 |
| `workflows.action-type.call-webhook` | Call Webhook | workflowFormTypes | — | in-pr4 |
| `workflows.action-type.delay` | Delay | workflowFormTypes | — | in-pr4 |
| `workflows.action-type.request-approval` | Request Approval | workflowFormTypes | — | in-pr4 |
| `workflows.action-type.send-dm` | Send DM | workflowFormTypes | — | in-pr4 |
| `workflows.action-type.send-message` | Send Message | workflowFormTypes | — | in-pr4 |
| `workflows.action-type.set-channel-topic` | Set Channel Topic | workflowFormTypes | — | in-pr4 |
| `workflows.activation.cron-every-hour` | It is scheduled to run every hour. Review the schedule before turning it on. | workflowActivationWarning | — | in-pr4 |
| `workflows.activation.cron-every-minute` | It is scheduled to run every minute. Review the schedule before turning it on. | workflowActivationWarning | — | in-pr4 |
| `workflows.activation.cron-every-n-minutes_one` | It is scheduled to run every {{count}} minute. Review the schedule before turning it on. | workflowActivationWarning | — | in-pr4 |
| `workflows.activation.cron-every-n-minutes_other` | It is scheduled to run every {{count}} minutes. Review the schedule before turning it on. | workflowActivationWarning | — | in-pr4 |
| `workflows.activation.cron-multiple-per-hour` | It is scheduled to run multiple times an hour. Review the schedule before turning it on. | workflowActivationWarning | — | in-pr4 |
| `workflows.activation.cron-multiple-per-minute` | It is scheduled to run multiple times a minute. Review the schedule before turning it on. | workflowActivationWarning | — | in-pr4 |
| `workflows.activation.frequent-interval` | It is scheduled to run every {{duration}}. Review the schedule before turning it on. | workflowActivationWarning | — | in-pr4 |
| `workflows.activation.frequent-title` | This workflow may run often | workflowActivationWarning | — | in-pr4 |
| `workflows.activation.message-all-description` | It will run for every new message in this channel. Review the trigger before turning it on. | workflowActivationWarning | — | in-pr4 |
| `workflows.approval.approver` | Approver: {{approver}} | WorkflowApprovalCard | — | in-pr4 |
| `workflows.approval.expires` | Expires: {{time}} | WorkflowApprovalCard | — | in-pr4 |
| `workflows.approval.title` | Approval Required | WorkflowApprovalCard | — | in-pr4 |
| `workflows.approval.unavailable` | Approval actions are not yet available in Desktop. | WorkflowApprovalCard | — | in-pr4 |
| `workflows.card.action-add-reaction` | add a reaction | workflowDefinition | — | in-pr4 |
| `workflows.card.action-add-reaction-detail` | add a {{detail}} reaction | workflowDefinition | — | in-pr4 |
| `workflows.card.action-call-webhook` | call a webhook | workflowDefinition | — | in-pr4 |
| `workflows.card.action-call-webhook-detail` | call {{detail}} | workflowDefinition | — | in-pr4 |
| `workflows.card.action-delay` | wait for a moment | workflowDefinition | — | in-pr4 |
| `workflows.card.action-delay-detail` | wait {{detail}} | workflowDefinition | — | in-pr4 |
| `workflows.card.action-request-approval` | request approval | workflowDefinition | — | in-pr4 |
| `workflows.card.action-request-approval-detail` | request approval: {{detail}} | workflowDefinition | — | in-pr4 |
| `workflows.card.action-send-dm` | send a direct message | workflowDefinition | — | in-pr4 |
| `workflows.card.action-send-dm-detail` | send {{detail}} | workflowDefinition | — | in-pr4 |
| `workflows.card.action-send-message` | send a channel message | workflowDefinition | — | in-pr4 |
| `workflows.card.action-send-text` | send {{detail}} | workflowDefinition | — | in-pr4 |
| `workflows.card.action-send-to-channel` | send a message in {{detail}} | workflowDefinition | — | in-pr4 |
| `workflows.card.action-set-topic` | update the channel topic | workflowDefinition | — | in-pr4 |
| `workflows.card.action-set-topic-detail` | set the channel topic to {{detail}} | workflowDefinition | — | in-pr4 |
| `workflows.card.label-with-action` | {{trigger}}, {{action}} | workflowDefinition | — | in-pr4 |
| `workflows.card.label-with-action-and-more_one` | {{trigger}}, {{action}}, then {{count}} more step | workflowDefinition | — | in-pr4 |
| `workflows.card.label-with-action-and-more_other` | {{trigger}}, {{action}}, then {{count}} more steps | workflowDefinition | — | in-pr4 |
| `workflows.card.schedule-custom-cron` | On a custom schedule | workflowDefinition | — | in-pr4 |
| `workflows.card.schedule-custom-interval` | Every {{interval}} | workflowDefinition | — | in-pr4 |
| `workflows.card.schedule-daily` | Every day at {{time}} UTC | workflowDefinition | — | in-pr4 |
| `workflows.card.schedule-monthly` | Every month at {{time}} UTC | workflowDefinition | — | in-pr4 |
| `workflows.card.schedule-on-a-schedule` | On a schedule | workflowDefinition | — | in-pr4 |
| `workflows.card.schedule-weekly` | Every week at {{time}} UTC | workflowDefinition | — | in-pr4 |
| `workflows.card.trigger-diff` | When a diff is posted | workflowDefinition | — | in-pr4 |
| `workflows.card.trigger-diff-matching` | When a matching diff is posted | workflowDefinition | — | in-pr4 |
| `workflows.card.trigger-fallback` | When this workflow starts | workflowDefinition | — | in-pr4 |
| `workflows.card.trigger-message` | When a message is posted | workflowDefinition | — | in-pr4 |
| `workflows.card.trigger-message-matching` | When a matching message is posted | workflowDefinition | — | in-pr4 |
| `workflows.card.trigger-reaction` | When someone adds a reaction | workflowDefinition | — | in-pr4 |
| `workflows.card.trigger-reaction-with` | When someone reacts with {{emoji}} | workflowDefinition | — | in-pr4 |
| `workflows.card.trigger-unknown` | When {{trigger}} happens | workflowDefinition | — | in-pr4 |
| `workflows.card.trigger-webhook` | When a webhook arrives | workflowDefinition | — | in-pr4 |
| `workflows.condition.error-hex-event-id` | Enter a 64-character hex event ID. | workflowConditionExpression | — | in-pr4 |
| `workflows.condition.error-hex-pubkey` | Enter a 64-character hex pubkey. | workflowConditionExpression | — | in-pr4 |
| `workflows.condition.field-author` | Author | workflowConditionExpression | — | in-pr4 |
| `workflows.condition.field-diff-text` | Diff text | workflowConditionExpression | — | in-pr4 |
| `workflows.condition.field-message` | Message | workflowConditionExpression | — | in-pr4 |
| `workflows.condition.field-message-text` | Message text | workflowConditionExpression | — | in-pr4 |
| `workflows.condition.field-reaction-emoji` | Reaction emoji | workflowConditionExpression | — | in-pr4 |
| `workflows.cron.error-empty-list-item` | {{field}} has an empty list item. | cronExpression | — | in-pr4 |
| `workflows.cron.error-invalid-range` | {{field}} has an invalid range. | cronExpression | — | in-pr4 |
| `workflows.cron.error-invalid-step` | {{field}} has an invalid step. | cronExpression | — | in-pr4 |
| `workflows.cron.error-out-of-range` | {{field}} must be between {{min}} and {{max}}. | cronExpression | — | in-pr4 |
| `workflows.cron.error-range-descending` | {{field}} range must go from lower to higher. | cronExpression | — | in-pr4 |
| `workflows.cron.error-required` | {{field}} is required. | cronExpression | — | in-pr4 |
| `workflows.cron.error-step-not-positive-integer` | {{field}} step must be a positive whole number. | cronExpression | — | in-pr4 |
| `workflows.cron.error-unsupported-value` | {{field}} contains “{{value}}”, which is not a supported value. | cronExpression | — | in-pr4 |
| `workflows.cron.field-day` | Day | cronExpression | — | in-pr4 |
| `workflows.cron.field-hour` | Hour | cronExpression | — | in-pr4 |
| `workflows.cron.field-minute` | Minute | cronExpression | — | in-pr4 |
| `workflows.cron.field-month` | Month | cronExpression | — | in-pr4 |
| `workflows.cron.field-weekday` | Weekday | cronExpression | — | in-pr4 |
| `workflows.cron.hint` | UTC · Paste all 5 fields, or use wildcards, lists, ranges, and steps. | CronExpressionInput | — | in-pr4 |
| `workflows.cron.legend` | Cron expression | CronExpressionInput | — | in-pr4 |
| `workflows.cron.paste-field-count_one` | Paste a 5-field cron expression. Found {{count}} field. | cronExpression | — | in-pr4 |
| `workflows.cron.paste-field-count_other` | Paste a 5-field cron expression. Found {{count}} fields. | cronExpression | — | in-pr4 |
| `workflows.delete.cancel` | Cancel | WorkflowDeleteDialog | — | in-pr4 |
| `workflows.delete.confirm` | Delete | WorkflowDeleteDialog | — | in-pr4 |
| `workflows.delete.deleting` | Deleting… | WorkflowDeleteDialog | — | in-pr4 |
| `workflows.delete.description` | Delete "{{name}}". This will stop all future triggers and remove the workflow permanently. | WorkflowDeleteDialog | — | in-pr4 |
| `workflows.delete.description-unnamed` | Delete this workflow. | WorkflowDeleteDialog | — | in-pr4 |
| `workflows.delete.error` | Couldn’t delete workflow. {{error}} Try again or cancel to keep editing. | WorkflowDeleteDialog | — | in-pr4 |
| `workflows.delete.title` | Delete workflow? | WorkflowDeleteDialog | — | in-pr4 |
| `workflows.dialog.continue` | Continue | WorkflowWebhookSecretDialog | — | in-pr4 |
| `workflows.duration.slider-aria` | {{label}} slider | WorkflowDurationField | — | in-pr4 |
| `workflows.duration.unit-day_one` | {{count}} day | workflowDuration | — | in-pr4 |
| `workflows.duration.unit-day_other` | {{count}} days | workflowDuration | — | in-pr4 |
| `workflows.duration.unit-hour_one` | {{count}} hour | workflowDuration | — | in-pr4 |
| `workflows.duration.unit-hour_other` | {{count}} hours | workflowDuration | — | in-pr4 |
| `workflows.duration.unit-minute_one` | {{count}} minute | workflowDuration | — | in-pr4 |
| `workflows.duration.unit-minute_other` | {{count}} minutes | workflowDuration | — | in-pr4 |
| `workflows.duration.unit-second_one` | {{count}} second | workflowDuration | — | in-pr4 |
| `workflows.duration.unit-second_other` | {{count}} seconds | workflowDuration | — | in-pr4 |
| `workflows.duration.unit-week_one` | {{count}} week | workflowDuration | — | in-pr4 |
| `workflows.duration.unit-week_other` | {{count}} weeks | workflowDuration | — | in-pr4 |
| `workflows.duration.zero-seconds` | 0 seconds | workflowDuration | — | in-pr4 |
| `workflows.emoji.choose` | Choose a reaction | WorkflowEmojiField | — | in-pr4 |
| `workflows.form.action` | Action | WorkflowFormBuilder | — | in-pr4 |
| `workflows.form.add-after` | Add after {{title}} | WorkflowFormBuilder | — | in-pr4 |
| `workflows.form.add-step` | Add step | WorkflowFormBuilder | — | in-pr4 |
| `workflows.form.cannot-switch-to-form` | Cannot switch to form view: {{error}} | WorkflowFormBuilder | — | in-pr4 |
| `workflows.form.close-inspector` | Close inspector | WorkflowFormBuilder | — | in-pr4 |
| `workflows.form.close-inspector-overlay` | Close inspector overlay | WorkflowFormBuilder | — | in-pr4 |
| `workflows.form.enable-workflow` | Enable workflow | WorkflowFormBuilder | — | in-pr4 |
| `workflows.form.node-inspector` | Workflow node inspector | WorkflowFormBuilder | — | in-pr4 |
| `workflows.form.remove-node` | Remove {{title}} | WorkflowFormBuilder | — | in-pr4 |
| `workflows.form.remove-step` | Remove step | WorkflowFormBuilder | — | in-pr4 |
| `workflows.form.sequence-aria` | Workflow sequence | WorkflowFormBuilder | — | in-pr4 |
| `workflows.form.step-node-label` | Step {{number}}: {{description}} | WorkflowFormBuilder | — | in-pr4 |
| `workflows.form.step-title` | Step {{number}} | WorkflowFormBuilder | — | in-pr4 |
| `workflows.form.trigger` | Trigger | WorkflowFormBuilder | — | in-pr4 |
| `workflows.form.trigger-event` | Trigger event | WorkflowFormBuilder | — | in-pr4 |
| `workflows.form.trigger-node-label` | Trigger: {{description}} | WorkflowFormBuilder | — | in-pr4 |
| `workflows.form.webhook-url-note` | A unique URL is generated after creation. | WorkflowFormBuilder | — | in-pr4 |
| `workflows.form.yaml-aria` | Workflow YAML | WorkflowFormBuilder | — | in-pr4 |
| `workflows.form.yaml-hint` | Edit the raw YAML definition directly. | WorkflowFormBuilder | — | in-pr4 |
| `workflows.schedule.custom-cron` | Custom cron | workflowSchedule | — | in-pr4 |
| `workflows.schedule.daily` | Daily | workflowSchedule | — | in-pr4 |
| `workflows.schedule.day-of-month` | Day of month | WorkflowScheduleFields | — | in-pr4 |
| `workflows.schedule.every-15-minutes` | Every 15 minutes | workflowSchedule | — | in-pr4 |
| `workflows.schedule.every-30-minutes` | Every 30 minutes | workflowSchedule | — | in-pr4 |
| `workflows.schedule.existing-interval` | Existing interval | WorkflowScheduleFields | — | in-pr4 |
| `workflows.schedule.hourly` | Every hour | workflowSchedule | — | in-pr4 |
| `workflows.schedule.interval-note` | Keep this legacy interval or choose a repeat option above. All schedules use UTC. | WorkflowScheduleFields | — | in-pr4 |
| `workflows.schedule.monthly` | Monthly | workflowSchedule | — | in-pr4 |
| `workflows.schedule.monthly-day-warning` | This schedule won’t run in some months. | WorkflowScheduleFields | — | in-pr4 |
| `workflows.schedule.repeat-on` | Repeat on | WorkflowScheduleFields | — | in-pr4 |
| `workflows.schedule.repeats` | Repeats | WorkflowScheduleFields | — | in-pr4 |
| `workflows.schedule.run-time` | Run time (UTC) | WorkflowScheduleFields | — | in-pr4 |
| `workflows.schedule.weekday-friday` | Friday | WorkflowScheduleFields | — | in-pr4 |
| `workflows.schedule.weekday-initial-friday` | F | WorkflowScheduleFields | — | in-pr4 |
| `workflows.schedule.weekday-initial-monday` | M | WorkflowScheduleFields | — | in-pr4 |
| `workflows.schedule.weekday-initial-saturday` | S | WorkflowScheduleFields | — | in-pr4 |
| `workflows.schedule.weekday-initial-sunday` | S | WorkflowScheduleFields | — | in-pr4 |
| `workflows.schedule.weekday-initial-thursday` | T | WorkflowScheduleFields | — | in-pr4 |
| `workflows.schedule.weekday-initial-tuesday` | T | WorkflowScheduleFields | — | in-pr4 |
| `workflows.schedule.weekday-initial-wednesday` | W | WorkflowScheduleFields | — | in-pr4 |
| `workflows.schedule.weekday-monday` | Monday | WorkflowScheduleFields | — | in-pr4 |
| `workflows.schedule.weekday-saturday` | Saturday | WorkflowScheduleFields | — | in-pr4 |
| `workflows.schedule.weekday-sunday` | Sunday | WorkflowScheduleFields | — | in-pr4 |
| `workflows.schedule.weekday-thursday` | Thursday | WorkflowScheduleFields | — | in-pr4 |
| `workflows.schedule.weekday-tuesday` | Tuesday | WorkflowScheduleFields | — | in-pr4 |
| `workflows.schedule.weekday-wednesday` | Wednesday | WorkflowScheduleFields | — | in-pr4 |
| `workflows.schedule.weekly` | Weekly | workflowSchedule | — | in-pr4 |
| `workflows.step-detail.from-approver` | {{message}} from {{approver}} | workflowStepDescription | — | in-pr4 |
| `workflows.step-detail.in-channel` | {{text}} in {{channel}} | workflowStepDescription | — | in-pr4 |
| `workflows.step-detail.to-recipient` | {{text}} to {{recipient}} | workflowStepDescription | — | in-pr4 |
| `workflows.step.duration` | Duration | WorkflowDurationField | — | in-pr4 |
| `workflows.template.author-pubkey` | Author pubkey | workflowTemplateVariables | — | in-pr4 |
| `workflows.template.channel-uuid` | Channel UUID | workflowTemplateVariables | — | in-pr4 |
| `workflows.template.diff-text` | Diff text | workflowTemplateVariables | — | in-pr4 |
| `workflows.template.group-previous-steps` | Previous steps | workflowTemplateVariables | — | in-pr4 |
| `workflows.template.group-trigger` | Trigger | workflowTemplateVariables | — | in-pr4 |
| `workflows.template.http-response-body` | HTTP response body | workflowTemplateVariables | — | in-pr4 |
| `workflows.template.http-response-status` | HTTP response status | workflowTemplateVariables | — | in-pr4 |
| `workflows.template.message-event-id` | Message event ID | workflowTemplateVariables | — | in-pr4 |
| `workflows.template.message-text` | Message text | workflowTemplateVariables | — | in-pr4 |
| `workflows.template.message-was-sent` | Whether the message was sent | workflowTemplateVariables | — | in-pr4 |
| `workflows.template.no-matching-variables` | No matching variables | WorkflowTemplateTextarea | — | in-pr4 |
| `workflows.template.reaction-emoji` | Reaction emoji | workflowTemplateVariables | — | in-pr4 |
| `workflows.template.reaction-was-added` | Whether the reaction was added | workflowTemplateVariables | — | in-pr4 |
| `workflows.template.seconds-elapsed` | Seconds elapsed | workflowTemplateVariables | — | in-pr4 |
| `workflows.template.sent-message-id` | Sent message ID | workflowTemplateVariables | — | in-pr4 |
| `workflows.template.unix-timestamp` | Unix timestamp | workflowTemplateVariables | — | in-pr4 |
| `workflows.template.webhook-hint` | JSON fields are available as {{field}}. | WorkflowTemplateTextarea | — | in-pr4 |
| `workflows.template.workflow-channel-uuid` | Workflow channel UUID | workflowTemplateVariables | — | in-pr4 |
| `workflows.text-condition.contains` | Contains | workflowMessageTextCondition | — | in-pr4 |
| `workflows.text-condition.ends-with` | Ends with | workflowMessageTextCondition | — | in-pr4 |
| `workflows.text-condition.equals` | Equals | workflowMessageTextCondition | — | in-pr4 |
| `workflows.text-condition.has-no-text` | Has no text | workflowMessageTextCondition | — | in-pr4 |
| `workflows.text-condition.has-text` | Has text | workflowMessageTextCondition | — | in-pr4 |
| `workflows.text-condition.not-contains` | Does not contain | workflowMessageTextCondition | — | in-pr4 |
| `workflows.text-condition.not-equals` | Does not equal | workflowMessageTextCondition | — | in-pr4 |
| `workflows.text-condition.starts-with` | Starts with | workflowMessageTextCondition | — | in-pr4 |
| `workflows.trace.error` | Error | WorkflowRunTrace | — | in-pr4 |
| `workflows.trace.no-steps` | No steps recorded yet. | WorkflowRunTrace | — | in-pr4 |
| `workflows.trace.output` | Output | WorkflowRunTrace | — | in-pr4 |
| `workflows.trace.pending-approval` | Pending approval | WorkflowRunTrace | — | in-pr4 |
| `workflows.trigger-type.diff-posted` | Diff Posted | workflowFormTypes | — | in-pr4 |
| `workflows.trigger-type.message-posted` | Message Posted | workflowFormTypes | — | in-pr4 |
| `workflows.trigger-type.reaction-added` | Reaction Added | workflowFormTypes | — | in-pr4 |
| `workflows.trigger-type.schedule` | Schedule | workflowFormTypes | — | in-pr4 |
| `workflows.trigger-type.webhook` | Webhook | workflowFormTypes | — | in-pr4 |
| `workflows.webhook.hide-secret` | Hide webhook secret | WorkflowWebhookSecretDialog | — | in-pr4 |
| `workflows.webhook.ready-description` | This private secret is shown once and cannot be recovered. Copy and store it before continuing. | WorkflowWebhookSecretDialog | — | in-pr4 |
| `workflows.webhook.ready-title` | Webhook ready | WorkflowWebhookSecretDialog | — | in-pr4 |
| `workflows.webhook.reveal-secret` | Reveal webhook secret | WorkflowWebhookSecretDialog | — | in-pr4 |
| `workflows.webhook.url-label` | Webhook URL | WorkflowWebhookSecretDialog | — | in-pr4 |
| `workflows.webhook.url-load-error` | The workflow was saved, but its webhook URL could not be loaded: {{error}} | WorkflowWebhookSecretDialog | — | in-pr4 |
| `workflows.webhook.url-loading` | Loading webhook URL… | WorkflowWebhookSecretDialog | — | in-pr4 |
| `workflows.yaml.append-use-yaml-editor` | {{error}} — use the YAML editor | workflowFormTypes | — | in-pr4 |
| `workflows.yaml.description-empty-or-padded` | description cannot be empty or have surrounding whitespace in Form mode — use the YAML editor | workflowFormTypes | — | in-pr4 |
| `workflows.yaml.duplicate-step-id` | Duplicate step ID "{{id}}" — use the YAML editor | workflowFormTypes | — | in-pr4 |
| `workflows.yaml.enabled-must-be-boolean` | enabled must be a boolean | workflowFormTypes | — | in-pr4 |
| `workflows.yaml.field-cannot-be-empty` | {{field}} cannot be empty in Form mode — use the YAML editor | workflowFormTypes | — | in-pr4 |
| `workflows.yaml.field-must-be-non-empty` | {{field}} must be a non-empty string | workflowFormTypes | — | in-pr4 |
| `workflows.yaml.field-must-be-string` | {{field}} must be a string | workflowFormTypes | — | in-pr4 |
| `workflows.yaml.invalid-yaml` | Invalid YAML | workflowFormTypes | — | in-pr4 |
| `workflows.yaml.must-be-object` | YAML must be an object | workflowFormTypes | — | in-pr4 |
| `workflows.yaml.reply-in-thread-unsupported` | reply_in_thread is not supported for {{trigger}} triggers — use the YAML editor | workflowFormTypes | — | in-pr4 |
| `workflows.yaml.schedule-both-cron-and-interval` | Schedule triggers cannot specify both cron and interval — use the YAML editor | workflowFormTypes | — | in-pr4 |
| `workflows.yaml.schedule-needs-cron-or-interval` | Schedule triggers require either cron or interval — use the YAML editor | workflowFormTypes | — | in-pr4 |
| `workflows.yaml.step-conditions-yaml-only` | Step conditions are only available in the YAML editor | workflowFormTypes | — | in-pr4 |
| `workflows.yaml.step-field` | Step {{number}} {{field}} | workflowFormTypes | — | in-pr4 |
| `workflows.yaml.step-id-invalid` | Step {{number}} requires a unique 1–64 character alphanumeric or underscore ID | workflowFormTypes | — | in-pr4 |
| `workflows.yaml.step-must-be-object` | Step {{number}} must be an object | workflowFormTypes | — | in-pr4 |
| `workflows.yaml.step-name-padded` | Step {{number}} name has surrounding whitespace — use the YAML editor | workflowFormTypes | — | in-pr4 |
| `workflows.yaml.step-reply-in-thread-must-be-boolean` | Step {{number}} reply_in_thread must be a boolean — use the YAML editor | workflowFormTypes | — | in-pr4 |
| `workflows.yaml.step-timeout-not-positive-integer` | Step {{number}} timeout_secs must be a positive integer | workflowFormTypes | — | in-pr4 |
| `workflows.yaml.steps-must-be-list` | steps must be a list | workflowFormTypes | — | in-pr4 |
| `workflows.yaml.trigger-on-required` | trigger.on is required | workflowFormTypes | — | in-pr4 |
| `workflows.yaml.unsupported-action-field` | Unsupported {{action}} step field "{{field}}" — use the YAML editor | workflowFormTypes | — | in-pr4 |
| `workflows.yaml.unsupported-action-type` | Unsupported action type "{{action}}" — use the YAML editor | workflowFormTypes | — | in-pr4 |
| `workflows.yaml.unsupported-cron` | Unsupported cron expression: {{error}} Use the YAML editor | workflowFormTypes | — | in-pr4 |
| `workflows.yaml.unsupported-trigger-field` | Unsupported {{trigger}} trigger field "{{field}}" — use the YAML editor | workflowFormTypes | — | in-pr4 |
| `workflows.yaml.unsupported-trigger-type` | Unsupported trigger type "{{trigger}}" — use the YAML editor | workflowFormTypes | — | in-pr4 |
| `workflows.yaml.unsupported-webhook-method` | Unsupported webhook method "{{method}}" — use the YAML editor | workflowFormTypes | — | in-pr4 |
| `workflows.yaml.unsupported-workflow-field` | Unsupported workflow field "{{field}}" — use the YAML editor | workflowFormTypes | — | in-pr4 |
| `workflows.yaml.use-yaml-editor-suffix` | use the YAML editor | workflowFormTypes | — | in-pr4 |
| `workflows.yaml.webhook-header-name-padded` | Webhook header names cannot be empty or have surrounding whitespace in Form mode | workflowFormTypes | — | in-pr4 |
| `workflows.yaml.webhook-headers-shape` | Webhook headers must be a non-empty object containing string values | workflowFormTypes | — | in-pr4 |

| `pulse.actions.copy-link-failed` | Failed to copy note link | useNoteActions | — | in-pr4 |
| `pulse.actions.link-copied` | Copied note link | useNoteActions | — | in-pr4 |
| `pulse.actions.open-dm-failed` | Failed to open DM | useNoteActions | — | in-pr4 |
| `pulse.actions.reaction-failed` | Failed to update reaction | useNoteActions | — | in-pr4 |
| `pulse.actions.reply-failed` | Failed to post reply | useNoteActions | — | in-pr4 |
| `pulse.card.like` | Like | NoteCard | — | in-pr4 |
| `pulse.card.loading-reply-context` | Loading reply context… | NoteCard | — | in-pr4 |
| `pulse.card.no-text` | No text | NoteCard | — | in-pr4 |
| `pulse.card.parent-note-author` | Parent note author | NoteCard | — | in-pr4 |
| `pulse.card.reply-placeholder` | Post your reply | NoteCard | — | in-pr4 |
| `pulse.card.replying-to-unavailable` | Replying to an unavailable note | NoteCard | — | in-pr4 |
| `pulse.card.start-direct-message` | Start direct message | NoteCard | — | in-pr4 |
| `pulse.card.unlike` | Unlike | NoteCard | — | in-pr4 |
| `pulse.search.submit-aria` | Search Pulse | PulseTabBar, PulseView | — | in-pr4 |
| `pulse.tabs.everyone` | Everyone | PulseTabBar | — | in-pr4 |
| `pulse.tabs.following` | Following | PulseTabBar | — | in-pr4 |
| `pulse.tabs.liked` | Liked | PulseTabBar | — | in-pr4 |
| `pulse.tabs.mine` | Mine | PulseTabBar | — | in-pr4 |
| `pulse.tabs.sections-aria` | Pulse sections | PulseTabBar | — | in-pr4 |
| `pulse.view.composer-placeholder` | What's on your mind? | PulseView | — | in-pr4 |
| `pulse.view.empty-agents-no-notes` | No agent notes yet. Agents post here when they publish. | PulseView | — | in-pr4 |
| `pulse.view.empty-agents-none-registered` | No agents registered yet. | PulseView | — | in-pr4 |
| `pulse.view.empty-everyone` | No public notes yet. | PulseView | — | in-pr4 |
| `pulse.view.empty-liked` | No likes yet — tap the heart on a note to save it here. | PulseView | — | in-pr4 |
| `pulse.view.empty-mine` | You haven't posted any notes yet. | PulseView | — | in-pr4 |
| `pulse.view.empty-people` | No notes yet. Follow people to see their updates here. | PulseView | — | in-pr4 |
| `pulse.view.empty-search` | Search Pulse notes by author or text. | PulseView | — | in-pr4 |
| `pulse.view.publish-failed` | Failed to publish note | PulseView | — | in-pr4 |
| `pulse.view.search-heading` | What are you looking for? | PulseView | — | in-pr4 |
| `pulse.view.search-placeholder` | What would you like to know? | PulseView | — | in-pr4 |

| `messages.attachment.huddle-description` | Start a huddle to talk to them. | WaveMessageAttachment | — | in-pr4 |
| `messages.attachment.start-huddle` | Start huddle | WaveMessageAttachment | — | in-pr4 |
| `messages.composer.skip` | Skip | ComposerUploadProgressOverlay | — | in-pr4 |
| `messages.delete.failed` | Failed to delete message: {{error}} | hooks | — | in-pr4 |
| `messages.diff.no-content` | No diff content | DiffViewer | — | in-pr4 |
| `messages.diff.no-textual-hunks` | No textual hunks in this diff. | DiffViewer | — | in-pr4 |
| `messages.mention.create-agent-mention-failed_one` | Could not create agent mention: {{error}} | useMentionSendFlow | — | in-pr4 |
| `messages.mention.create-agent-mention-failed_other` | Could not create agent mentions: {{error}} | useMentionSendFlow | — | in-pr4 |
| `messages.mention.prepare-agent-mention-failed_one` | Could not prepare agent mention: {{error}} | useMentionSendFlow | — | in-pr4 |
| `messages.mention.prepare-agent-mention-failed_other` | Could not prepare agent mentions: {{error}} | useMentionSendFlow | — | in-pr4 |
| `messages.thread.no-replies` | No replies in this branch yet | MessageThreadReplyState | — | in-pr4 |
| `messages.thread.no-replies-hint` | Reply in the thread to continue this branch. | MessageThreadReplyState | — | in-pr4 |
| `messages.thread.open-title` | Open {{title}} | MessageThreadPanelSkeleton | — | in-pr4 |
| `messages.thread.retry` | Retry | MessageThreadReplyState | — | in-pr4 |
| `messages.thread.summary-last-reply` | last reply | MessageThreadSummaryRow | — | in-pr4 |
| `messages.thread.view-thread` | View thread | MessageThreadSummaryRow | — | in-pr4 |
| `messages.timeline.channel-intro-prefix` | This is the beginning of the | ChannelIntroBlock | — | in-pr4 |
| `messages.timeline.channel-intro-suffix` | . | ChannelIntroBlock | — | in-pr4 |
| `messages.timeline.retry` | Retry | MessageTimelineErrorCard | — | in-pr4 |
| `sidebar.activity.open-thread-from` | Open thread from {{name}} | ChannelActivityPopover | — | in-pr4 |
| `sidebar.activity.working-status` | Working | ChannelActivityPopover | — | in-pr4 |
| `sidebar.channel-dialog.create-new` | Create a new {{kind}} | CreateChannelDialog | — | in-pr4 |
| `sidebar.channel-form.create-kind` | Create {{kind}} | CreateChannelFormFields | — | in-pr4 |
| `sidebar.channel-form.creating` | Creating... | CreateChannelFormFields | — | in-pr4 |
| `sidebar.channel-form.description` | Description | CreateChannelFormFields | — | in-pr4 |
| `sidebar.channel-form.description-placeholder` | What this {{kind}} is for | CreateChannelFormFields | — | in-pr4 |
| `sidebar.channel-form.name` | Name | CreateChannelFormFields | — | in-pr4 |
| `sidebar.channel-form.new-template` | Create new channel template… | CreateChannelFormFields | — | in-pr4 |
| `sidebar.channel.confirm-leave` | Leave | ChannelSectionDialogs | — | in-pr4 |
| `sidebar.channel.leave-body` | Leave "{{channelName}}"? You'll stop receiving its messages and can rejoin later. | ChannelSectionDialogs | — | in-pr4 |
| `sidebar.profile-card.open-menu-aria` | Open profile menu for {{name}} | SidebarProfileCard | — | in-pr4 |
| `sidebar.projects.delete-aria` | Delete {{name}} | SidebarProjectsSection | — | in-pr4 |
| `sidebar.projects.delete-body` | Delete {{name}} from Projects for everyone. This can only be done for projects you own and cannot be undone. | SidebarProjectsSection | — | in-pr4 |
| `sidebar.projects.deleting` | Deleting... | SidebarProjectsSection | — | in-pr4 |
| `sidebar.projects.empty` | No projects yet | SidebarProjectsSection | — | in-pr4 |
| `sidebar.projects.filter-added` | Added | SidebarProjectsSection | — | in-pr4 |
| `sidebar.projects.filter-owned` | Owned by me | SidebarProjectsSection | — | in-pr4 |
| `sidebar.projects.hide-channels` | Hide channels in {{name}} | SidebarProjectsSection | — | in-pr4 |
| `sidebar.projects.show-channels` | Show channels in {{name}} | SidebarProjectsSection | — | in-pr4 |
| `sidebar.projects.sort-newest` | Newest | SidebarProjectsSection | — | in-pr4 |
| `sidebar.rail.copy-url` | Copy community URL | CommunityRail | — | in-pr4 |
| `sidebar.rail.invite` | Invite to community | CommunityRail | — | in-pr4 |
| `sidebar.rail.settings` | Community settings | CommunityRail | — | in-pr4 |
| `sidebar.sections.add-channel-to` | Add channel to {{section}} | CustomChannelSection | — | in-pr4 |
| `sidebar.sections.confirm-create` | Create | ChannelSectionDialogs | — | in-pr4 |
| `sidebar.sections.confirm-delete` | Delete | ChannelSectionDialogs | — | in-pr4 |
| `sidebar.sections.confirm-save` | Save | ChannelSectionDialogs | — | in-pr4 |
| `sidebar.sections.create-description` | Sections let you group related channels in the sidebar. | ChannelSectionDialogs | — | in-pr4 |
| `sidebar.sections.more-actions-for` | More actions for {{section}} | CustomChannelSection | — | in-pr4 |
| `sidebar.sections.rename-description` | Enter a new name for this section. | ChannelSectionDialogs | — | in-pr4 |

| `settings.backup.another-copy-description` | You can download another copy for 5 minutes. | EncryptedBackupProvider | — | in-pr4 |
| `settings.backup.available-description` | Your backup will be available to download for 5 minutes. | EncryptedBackupProvider | — | in-pr4 |
| `settings.backup.create-action` | Backup key | EncryptedBackupCreator | — | in-pr4 |
| `settings.backup.create-backup-action` | Create backup | PrivateKeyBackupRow | — | in-pr4 |
| `settings.backup.create-description` | You can close this window while Buzz finishes the backup in the background. | EncryptedBackupCreator | — | in-pr4 |
| `settings.backup.create-failed-title` | Couldn’t create backup | EncryptedBackupProvider | — | in-pr4 |
| `settings.backup.create-title` | Create a key backup | EncryptedBackupCreator | — | in-pr4 |
| `settings.backup.create-warning` | Keep the file private and save its password somewhere safe — Buzz cannot reset it. Once ready, the backup remains available to download for 5 minutes. | EncryptedBackupCreator | — | in-pr4 |
| `settings.backup.download-backup` | Download backup | PrivateKeyBackupRow | — | in-pr4 |
| `settings.backup.hide` | Hide | PrivateKeyBackupRow | — | in-pr4 |
| `settings.backup.open-settings` | Open settings | EncryptedBackupProvider | — | in-pr4 |
| `settings.backup.preparing-description` | You can close this window while Buzz finishes. | EncryptedBackupProvider | — | in-pr4 |
| `settings.backup.preparing-title` | Preparing backup… | EncryptedBackupProvider | — | in-pr4 |
| `settings.backup.ready-title` | Backup ready to download | EncryptedBackupProvider | — | in-pr4 |
| `settings.backup.reveal` | Reveal | PrivateKeyBackupRow | — | in-pr4 |
| `settings.backup.save-error-description` | {{message}} It will be available to download for 5 minutes. | EncryptedBackupProvider | — | in-pr4 |
| `settings.backup.saving-description` | The download window will open when it’s ready. | EncryptedBackupProvider | — | in-pr4 |
| `settings.backup.saving-title` | Saving backup… | EncryptedBackupProvider | — | in-pr4 |
| `settings.backup.test-backup-action` | Test backup | PrivateKeyBackupRow | — | in-pr4 |
| `settings.backup.test-description` | Confirm that a backup file and its password can unlock an identity. | PrivateKeyBackupRow | — | in-pr4 |
| `settings.backup.test-format-note` | Backups use the standard NIP-49 format, so this works for backups from compatible Nostr apps too. | PrivateKeyBackupRow | — | in-pr4 |
| `settings.backup.test-hint` | That's the one. Now enter your password to prove you can unlock it. | BackupTestFlow | — | in-pr4 |
| `settings.backup.test-title` | Test a key backup | PrivateKeyBackupRow | — | in-pr4 |
| `settings.backup.use-different-file` | Use a different file | BackupTestFlow | — | in-pr4 |
| `settings.updates.available` | Update available | SidebarUpdateCard | — | in-pr4 |
| `settings.updates.available-version` | Update available — v{{version}} | UpdateChecker | — | in-pr4 |
| `settings.updates.card-click-to-update` | Click to update | SidebarUpdateCard | — | in-pr4 |
| `settings.updates.card-ready-title` | Ready to update! | SidebarUpdateCard | — | in-pr4 |
| `settings.updates.card-updating` | Updating | SidebarUpdateCard | — | in-pr4 |
| `settings.updates.check` | Check for Updates | UpdateChecker | — | in-pr4 |
| `settings.updates.check-again` | Check Again | UpdateChecker | — | in-pr4 |
| `settings.updates.checking` | Checking for updates... | UpdateChecker | — | in-pr4 |
| `settings.updates.dismiss` | Dismiss update notification | SidebarUpdateCard | — | in-pr4 |
| `settings.updates.download-github` | Download update from GitHub | SidebarUpdateCard | — | in-pr4 |
| `settings.updates.download-update` | Download Update | UpdateChecker | — | in-pr4 |
| `settings.updates.downloading-dots` | Downloading update... | UpdateChecker | — | in-pr4 |
| `settings.updates.failed` | Update failed: {{message}} | UpdateChecker | — | in-pr4 |
| `settings.updates.idle-hint` | Check if a new version is available. | UpdateChecker | — | in-pr4 |
| `settings.updates.indicator-downloading` | Downloading update… | settings surface | — | in-pr4 |
| `settings.updates.indicator-installing` | Installing update… | settings surface | — | in-pr4 |
| `settings.updates.indicator-manual-required` | Update available — download from GitHub (use AppImage for auto-updates) | settings surface | — | in-pr4 |
| `settings.updates.installing-dots` | Installing update... | UpdateChecker | — | in-pr4 |
| `settings.updates.manual-required-appimage` | Switch to the AppImage build for automatic updates. | UpdateChecker | — | in-pr4 |
| `settings.updates.manual-required-description` | v{{version}} available — download from GitHub. Switch to AppImage for automatic updates. | SidebarUpdateCard | — | in-pr4 |
| `settings.updates.manual-required-hint` | In-app updates aren't supported on this Linux package. Download the new version from GitHub. | UpdateChecker | — | in-pr4 |
| `settings.updates.preparing` | Preparing update... | UpdateChecker | — | in-pr4 |
| `settings.updates.ready-hint` | Update downloaded. Click to apply. | UpdateChecker | — | in-pr4 |
| `settings.updates.retry` | Retry | UpdateChecker | — | in-pr4 |
| `settings.updates.section-description` | Keep Buzz up to date with the latest features and fixes. | UpdateChecker | — | in-pr4 |
| `settings.updates.software-updates` | Software Updates | UpdateChecker | — | in-pr4 |
| `settings.updates.status-group` | Update status | UpdateChecker | — | in-pr4 |
| `settings.updates.unavailable-hint` | Automatic updates aren't available on this build. Download the latest release manually. | UpdateChecker | — | in-pr4 |
| `settings.updates.up-to-date` | You're on the latest version. | UpdateChecker | — | in-pr4 |
| `settings.updates.update-now` | Update Now | UpdateChecker | — | in-pr4 |
| `settings.updates.update-now-label` | Update now | SidebarUpdateCard | — | in-pr4 |

| `settings.agent-defaults.description` | Provider, model, effort, and environment settings inherited by local agents. Agent-specific settings always take priority. | AgentDefaultsSettingsCard | — | in-pr4 |
| `settings.agent-defaults.title` | Agent defaults | AgentDefaultsSettingsCard | — | in-pr4 |
| `settings.agents.auto-mention` | Automatically mention agents | AgentsSettingsPanel | — | in-pr4 |
| `settings.agents.auto-mention-hint` | Address selected agents in thread replies | AgentsSettingsPanel | — | in-pr4 |
| `settings.agents.description` | Control how agents behave in conversations and run on this machine. | AgentsSettingsPanel | — | in-pr4 |
| `settings.agents.group-conversations` | Conversations | AgentsSettingsPanel | — | in-pr4 |
| `settings.agents.title` | Agents | AgentsSettingsPanel | — | in-pr4 |
| `settings.experiments.description` | These features are functional but still being refined. Enable them to try new capabilities early. | ExperimentalFeaturesCard | — | in-pr4 |
| `settings.experiments.group-features` | Features | ExperimentalFeaturesCard | — | in-pr4 |
| `settings.experiments.title` | Experiments | ExperimentalFeaturesCard | — | in-pr4 |
| `settings.notifications.blocked-error` | Desktop notifications are blocked. Enable them in your system settings. | NotificationSettingsCard | — | in-pr4 |
| `settings.notifications.coming-soon` | Coming soon | NotificationSettingsCard | — | in-pr4 |
| `settings.notifications.description` | Desktop alerts are on by default. Fine-tune what gets through below. | NotificationSettingsCard | — | in-pr4 |
| `settings.notifications.desktop-alerts` | Desktop alerts | NotificationSettingsCard | — | in-pr4 |
| `settings.notifications.desktop-disabled-hint` | Request OS permission and surface new mentions or needs-action items outside the app. | NotificationSettingsCard | — | in-pr4 |
| `settings.notifications.desktop-enabled-hint` | Native desktop alerts are enabled for the categories you have armed below. | NotificationSettingsCard | — | in-pr4 |
| `settings.notifications.group-alert-sounds` | Alert sounds | NotificationSettingsCard | — | in-pr4 |
| `settings.notifications.group-badges` | Badges | NotificationSettingsCard | — | in-pr4 |
| `settings.notifications.group-desktop` | Desktop | NotificationSettingsCard | — | in-pr4 |
| `settings.notifications.home-badge` | Home badge | NotificationSettingsCard | — | in-pr4 |
| `settings.notifications.home-badge-hint` | Show a Home badge for mentions and needs-action items in the sidebar. | NotificationSettingsCard | — | in-pr4 |
| `settings.notifications.notify-while-viewing` | Notify while viewing | NotificationSettingsCard | — | in-pr4 |
| `settings.notifications.notify-while-viewing-hint` | Also alert for direct messages in the conversation you have open. | NotificationSettingsCard | — | in-pr4 |
| `settings.notifications.requesting` | Requesting... | NotificationSettingsCard | — | in-pr4 |
| `settings.notifications.show-less` | Show less | NotificationSettingsCard | — | in-pr4 |
| `settings.notifications.sound` | Sound | NotificationSettingsCard | — | in-pr4 |
| `settings.notifications.sound-hint` | Alert with a sound for the events below. | NotificationSettingsCard | — | in-pr4 |
| `settings.notifications.state-blocked` | Blocked | NotificationSettingsCard | — | in-pr4 |
| `settings.notifications.state-off` | Off | NotificationSettingsCard | — | in-pr4 |
| `settings.notifications.state-on` | On | NotificationSettingsCard | — | in-pr4 |
| `settings.notifications.state-unavailable` | Unavailable | NotificationSettingsCard | — | in-pr4 |
| `settings.notifications.title` | Notifications | NotificationSettingsCard | — | in-pr4 |
| `settings.notifications.unsupported-error` | Desktop notifications are not supported in this environment. | NotificationSettingsCard | — | in-pr4 |
| `settings.notifications.view-all` | View all | NotificationSettingsCard | — | in-pr4 |
| `settings.signout.backup-confirm` | I have tested a key backup or saved this private key somewhere safe. | SignOutSection | — | in-pr4 |
| `settings.signout.backup-step` | 1. Confirm you can restore your identity | SignOutSection | — | in-pr4 |
| `settings.signout.delete-button` | Delete my data | SignOutSection | — | in-pr4 |
| `settings.signout.description` | Removes your identity key and all local app data from this device. Before signing out, create and test a password-protected key backup above — this cannot be undone. | SignOutSection | — | in-pr4 |
| `settings.signout.dialog-description` | This will delete your identity key, all agent settings, and cached data from this device, then relaunch Buzz into first-run setup. This cannot be undone. | SignOutSection | — | in-pr4 |
| `settings.signout.dialog-title` | Sign out and wipe all data? | SignOutSection | — | in-pr4 |
| `settings.signout.loading` | Loading… | SignOutSection | — | in-pr4 |
| `settings.signout.nsec-load-failed` | Failed to retrieve private key. | SignOutSection | — | in-pr4 |
| `settings.signout.pending` | Signing out… | SignOutSection | — | in-pr4 |
| `settings.signout.spinner-aria` | Signing out | SignOutSection | — | in-pr4 |
| `settings.signout.submit-failed` | Sign out failed. | SignOutSection | — | in-pr4 |
| `settings.signout.title` | Sign out | SignOutSection | — | in-pr4 |
| `settings.signout.type-prefix` | 2. Type | SignOutSection | — | in-pr4 |
| `settings.signout.type-suffix` | to confirm | SignOutSection | — | in-pr4 |
| `settings.voice.add-voice` | Add voice | VoiceSettingsCard | — | in-pr4 |
| `settings.voice.agent-tts` | Agent text to speech | VoiceSettingsCard | — | in-pr4 |
| `settings.voice.agent-tts-hint` | Read new agent messages aloud in the order they arrive. | VoiceSettingsCard | — | in-pr4 |
| `settings.voice.delete-aria` | Delete {{voice}} | VoiceSettingsCard | — | in-pr4 |
| `settings.voice.delete-confirm` | Delete voice | VoiceSettingsCard | — | in-pr4 |
| `settings.voice.delete-description` | {{name}} and its local audio file will be removed. | VoiceSettingsCard | — | in-pr4 |
| `settings.voice.delete-description-unnamed` | This imported voice and its local audio file will be removed. | VoiceSettingsCard | — | in-pr4 |
| `settings.voice.delete-failed` | Voice could not be deleted. | VoiceSettingsCard | — | in-pr4 |
| `settings.voice.delete-fallback-note` |  Mary will be selected instead. | VoiceSettingsCard | — | in-pr4 |
| `settings.voice.delete-title` | Delete imported voice? | VoiceSettingsCard | — | in-pr4 |
| `settings.voice.description` | Choose whether Buzz reads new agent responses aloud during an active huddle. | VoiceSettingsCard | — | in-pr4 |
| `settings.voice.group-playback` | Playback | VoiceSettingsCard | — | in-pr4 |
| `settings.voice.import-failed` | Voice could not be imported. | VoiceSettingsCard | — | in-pr4 |
| `settings.voice.load-failed` | Voice settings could not be loaded. | VoiceSettingsCard | — | in-pr4 |
| `settings.voice.pocket-voice` | Pocket TTS voice | VoiceSettingsCard | — | in-pr4 |
| `settings.voice.pocket-voice-hint` | Voice files stay private on this device. | VoiceSettingsCard | — | in-pr4 |
| `settings.voice.preview` | Preview | VoiceSettingsCard | — | in-pr4 |
| `settings.voice.preview-aria` | Preview {{voice}} | VoiceSettingsCard | — | in-pr4 |
| `settings.voice.preview-failed` | Voice preview could not be played. | VoiceSettingsCard | — | in-pr4 |
| `settings.voice.save-failed` | Voice settings could not be saved. | VoiceSettingsCard | — | in-pr4 |
| `settings.voice.selector-aria` | Pocket TTS voice: {{voice}} | VoiceSettingsCard | — | in-pr4 |
| `settings.voice.title` | Voice | VoiceSettingsCard | — | in-pr4 |

| `messages.link.thread-in-prefix` | Thread in | messages surface | — | in-pr4 |

| `shared.app-shell.error-body` | Reload Buzz to try again. If this keeps happening, check that Buzz can access website data, then contact support. | RootErrorBoundary | — | in-pr4 |
| `shared.app-shell.error-title` | Buzz failed to start | RootErrorBoundary | — | in-pr4 |
| `shared.app-shell.go-back` | Go back | AppTopChrome | — | in-pr4 |
| `shared.app-shell.go-forward` | Go forward | AppTopChrome | — | in-pr4 |
| `shared.app-shell.reload` | Reload | RootErrorBoundary | — | in-pr4 |
| `shared.app-shell.setting-up-community` | Setting up your community... | App | — | in-pr4 |
| `shared.app-shell.switching-community` | Switching community… | App | — | in-pr4 |
| `shared.app-shell.toggle-sidebar` | Toggle Sidebar | AppTopChrome | — | in-pr4 |
| `shared.clipboard.copied` | Copied to clipboard | clipboard | — | in-pr4 |
| `shared.clipboard.copy-failed` | Failed to copy to clipboard | clipboard | — | in-pr4 |
| `shared.keyboard.always-address-agent.description` | Address the default agent, or toggle the highlighted agent | keyboard-shortcuts | — | in-pr4 |
| `shared.keyboard.always-address-agent.label` | Always address agent | keyboard-shortcuts | — | in-pr4 |
| `shared.keyboard.browse-channels.description` | Open the channel browser | keyboard-shortcuts | — | in-pr4 |
| `shared.keyboard.browse-channels.label` | Browse channels | keyboard-shortcuts | — | in-pr4 |
| `shared.keyboard.browse-dms.description` | Open the new message composer | keyboard-shortcuts | — | in-pr4 |
| `shared.keyboard.browse-dms.label` | New direct message | keyboard-shortcuts | — | in-pr4 |
| `shared.keyboard.category.formatting` | Formatting | keyboard-shortcuts | — | in-pr4 |
| `shared.keyboard.category.messages` | Messages | keyboard-shortcuts | — | in-pr4 |
| `shared.keyboard.category.navigation` | Navigation | keyboard-shortcuts | — | in-pr4 |
| `shared.keyboard.category.zoom` | Zoom | keyboard-shortcuts | — | in-pr4 |
| `shared.keyboard.close-dialog.description` | Close the current dialog or settings | keyboard-shortcuts | — | in-pr4 |
| `shared.keyboard.close-dialog.label` | Close dialog | keyboard-shortcuts | — | in-pr4 |
| `shared.keyboard.find-in-channel.description` | Search messages in current channel | keyboard-shortcuts | — | in-pr4 |
| `shared.keyboard.find-in-channel.label` | Find in channel | keyboard-shortcuts | — | in-pr4 |
| `shared.keyboard.format-bold.description` | Toggle bold formatting | keyboard-shortcuts | — | in-pr4 |
| `shared.keyboard.format-bold.label` | Bold | keyboard-shortcuts | — | in-pr4 |
| `shared.keyboard.format-code.description` | Toggle inline code formatting | keyboard-shortcuts | — | in-pr4 |
| `shared.keyboard.format-code.label` | Inline code | keyboard-shortcuts | — | in-pr4 |
| `shared.keyboard.format-italic.description` | Toggle italic formatting | keyboard-shortcuts | — | in-pr4 |
| `shared.keyboard.format-italic.label` | Italic | keyboard-shortcuts | — | in-pr4 |
| `shared.keyboard.format-link.description` | Link the selected composer text, or edit the link under the caret | keyboard-shortcuts | — | in-pr4 |
| `shared.keyboard.format-link.label` | Insert link | keyboard-shortcuts | — | in-pr4 |
| `shared.keyboard.format-strikethrough.description` | Toggle strikethrough formatting | keyboard-shortcuts | — | in-pr4 |
| `shared.keyboard.format-strikethrough.label` | Strikethrough | keyboard-shortcuts | — | in-pr4 |
| `shared.keyboard.go-back.description` | Navigate to the previous page | keyboard-shortcuts | — | in-pr4 |
| `shared.keyboard.go-back.label` | Go back | keyboard-shortcuts | — | in-pr4 |
| `shared.keyboard.go-forward.description` | Navigate to the next page | keyboard-shortcuts | — | in-pr4 |
| `shared.keyboard.go-forward.label` | Go forward | keyboard-shortcuts | — | in-pr4 |
| `shared.keyboard.go-home.description` | Navigate to the home feed | keyboard-shortcuts | — | in-pr4 |
| `shared.keyboard.go-home.label` | Home | keyboard-shortcuts | — | in-pr4 |
| `shared.keyboard.mark-all-read.description` | Mark all conversations as read | keyboard-shortcuts | — | in-pr4 |
| `shared.keyboard.mark-all-read.label` | Mark all as read | keyboard-shortcuts | — | in-pr4 |
| `shared.keyboard.mark-current-read.description` | Mark the current conversation as read | keyboard-shortcuts | — | in-pr4 |
| `shared.keyboard.mark-current-read.label` | Mark as read | keyboard-shortcuts | — | in-pr4 |
| `shared.keyboard.new-channel.description` | Open the create channel dialog | keyboard-shortcuts | — | in-pr4 |
| `shared.keyboard.new-channel.label` | New channel | keyboard-shortcuts | — | in-pr4 |
| `shared.keyboard.new-line.description` | Insert a line break in the composer | keyboard-shortcuts | — | in-pr4 |
| `shared.keyboard.new-line.label` | New line | keyboard-shortcuts | — | in-pr4 |
| `shared.keyboard.open-settings.description` | Open or close settings | keyboard-shortcuts | — | in-pr4 |
| `shared.keyboard.open-settings.label` | Settings | keyboard-shortcuts | — | in-pr4 |
| `shared.keyboard.publish-note.description` | Publish a Pulse note | keyboard-shortcuts | — | in-pr4 |
| `shared.keyboard.publish-note.label` | Publish note | keyboard-shortcuts | — | in-pr4 |
| `shared.keyboard.push-to-talk.description` | Hold to unmute in a huddle | keyboard-shortcuts | — | in-pr4 |
| `shared.keyboard.push-to-talk.label` | Push to talk | keyboard-shortcuts | — | in-pr4 |
| `shared.keyboard.quick-search.description` | Open the search dialog | keyboard-shortcuts | — | in-pr4 |
| `shared.keyboard.quick-search.label` | Quick search | keyboard-shortcuts | — | in-pr4 |
| `shared.keyboard.send-message.description` | Send the current message | keyboard-shortcuts | — | in-pr4 |
| `shared.keyboard.send-message.label` | Send message | keyboard-shortcuts | — | in-pr4 |
| `shared.keyboard.toggle-huddle.description` | Start or join a huddle in the current channel; leave when connected | keyboard-shortcuts | — | in-pr4 |
| `shared.keyboard.toggle-huddle.label` | Start or leave huddle | keyboard-shortcuts | — | in-pr4 |
| `shared.keyboard.toggle-sidebar.description` | Show or hide the sidebar | keyboard-shortcuts | — | in-pr4 |
| `shared.keyboard.toggle-sidebar.label` | Toggle sidebar | keyboard-shortcuts | — | in-pr4 |
| `shared.keyboard.zoom-in.description` | Increase the zoom level | keyboard-shortcuts | — | in-pr4 |
| `shared.keyboard.zoom-in.label` | Zoom in | keyboard-shortcuts | — | in-pr4 |
| `shared.keyboard.zoom-out.description` | Decrease the zoom level | keyboard-shortcuts | — | in-pr4 |
| `shared.keyboard.zoom-out.label` | Zoom out | keyboard-shortcuts | — | in-pr4 |
| `shared.keyboard.zoom-reset.description` | Reset zoom to default level | keyboard-shortcuts | — | in-pr4 |
| `shared.keyboard.zoom-reset.label` | Reset zoom | keyboard-shortcuts | — | in-pr4 |

| `customEmoji.add-group.title` | Add emoji | CustomEmojiSettingsCard | — | in-pr4 |
| `customEmoji.add.added` | Added :{{shortcode}}: | CustomEmojiSettingsCard | — | in-pr4 |
| `customEmoji.add.failed` | Failed to add emoji. | CustomEmojiSettingsCard | — | in-pr4 |
| `customEmoji.card.description-prefix` | Add your own custom emoji for everyone on this relay to use. Type | CustomEmojiSettingsCard | — | in-pr4 |
| `customEmoji.card.description-suffix` | in messages and reactions. | CustomEmojiSettingsCard | — | in-pr4 |
| `customEmoji.card.title` | Custom emoji | CustomEmojiSettingsCard | — | in-pr4 |
| `customEmoji.community.description` | Added by other members. You can use these, but only their owner can remove them. | CustomEmojiSettingsCard | — | in-pr4 |
| `customEmoji.community.title-count` | Community emoji ({{count}}) | CustomEmojiSettingsCard | — | in-pr4 |
| `customEmoji.form.clear` | Clear | CustomEmojiSettingsCard | — | in-pr4 |
| `customEmoji.form.save` | Save emoji | CustomEmojiSettingsCard | — | in-pr4 |
| `customEmoji.form.saving` | Saving… | CustomEmojiSettingsCard | — | in-pr4 |
| `customEmoji.mine.empty` | You haven't added any emoji yet. Add one above. | CustomEmojiSettingsCard | — | in-pr4 |
| `customEmoji.mine.loading` | Loading… | CustomEmojiSettingsCard | — | in-pr4 |
| `customEmoji.mine.remove-aria` | Remove :{{shortcode}}: | CustomEmojiSettingsCard | — | in-pr4 |
| `customEmoji.mine.title` | My emoji | CustomEmojiSettingsCard | — | in-pr4 |
| `customEmoji.mine.title-count` | My emoji ({{count}}) | CustomEmojiSettingsCard | — | in-pr4 |
| `customEmoji.name.choose-first` | Choose an image first; Buzz will suggest a name from the filename. | CustomEmojiSettingsCard | — | in-pr4 |
| `customEmoji.name.heading` | Give it a name | CustomEmojiSettingsCard | — | in-pr4 |
| `customEmoji.name.hint` | This is what you’ll type to add this emoji to messages and reactions. | CustomEmojiSettingsCard | — | in-pr4 |
| `customEmoji.name.invalid` | Use only letters, numbers, hyphen, or underscore. | CustomEmojiSettingsCard | — | in-pr4 |
| `customEmoji.name.replaces` | You already have :{{shortcode}}: — saving will replace its image. | CustomEmojiSettingsCard | — | in-pr4 |
| `customEmoji.remove.failed` | Failed to remove emoji. | CustomEmojiSettingsCard | — | in-pr4 |
| `customEmoji.remove.removed` | Removed :{{shortcode}}: | CustomEmojiSettingsCard | — | in-pr4 |
| `customEmoji.upload.button` | Upload image | CustomEmojiSettingsCard | — | in-pr4 |
| `customEmoji.upload.choose-different` | Choose different image | CustomEmojiSettingsCard | — | in-pr4 |
| `customEmoji.upload.heading` | Upload an image | CustomEmojiSettingsCard | — | in-pr4 |
| `customEmoji.upload.hint` | Square images work best. GIF, PNG, JPEG, and WebP files are supported. | CustomEmojiSettingsCard | — | in-pr4 |
| `customEmoji.upload.image-failed` | Failed to upload emoji image. | CustomEmojiSettingsCard | — | in-pr4 |
| `customEmoji.upload.not-image` | Choose an image file for custom emoji. | CustomEmojiSettingsCard | — | in-pr4 |
| `customEmoji.upload.preview-alt` | Selected custom emoji preview | CustomEmojiSettingsCard | — | in-pr4 |
| `customEmoji.upload.uploading` | Uploading… | CustomEmojiSettingsCard | — | in-pr4 |
| `presence.status.away` | Away | presence | — | in-pr4 |
| `presence.status.offline` | Offline | presence | — | in-pr4 |
| `presence.status.online` | Online | presence | — | in-pr4 |

| `communities.add.back-aria` | Back to add community options | AddCommunityDialog | — | in-pr4 |
| `communities.add.choose-description` | Create a new community or join one you already have. | AddCommunityDialog | — | in-pr4 |
| `communities.add.choose-title` | Add community | AddCommunityDialog | — | in-pr4 |
| `communities.add.create-description` | Opens Builderlab in your browser. | AddCommunityDialog | — | in-pr4 |
| `communities.add.create-option-hint` | Claim a Buzz address for your team. | AddCommunityDialog | — | in-pr4 |
| `communities.add.create-title` | Create a new community | AddCommunityDialog | — | in-pr4 |
| `communities.add.join-description` | Use the community URL or invite link you received. | AddCommunityDialog | — | in-pr4 |
| `communities.add.join-error-busy` | Finish connecting the community already in progress, then try again. | AddCommunityDialog, HostedCommunityCreateFlow | — | in-pr4 |
| `communities.add.join-option-hint` | Use a community URL or invite link. | AddCommunityDialog | — | in-pr4 |
| `communities.add.join-title` | Join an existing community | AddCommunityDialog | — | in-pr4 |
| `communities.api-error.identity-already-bound` | This Builderlab account is connected to another Buzz identity. | hostedCommunityApi | — | in-pr4 |
| `communities.api-error.invalid-name` | Use lowercase letters, numbers, and hyphens. | hostedCommunityApi | — | in-pr4 |
| `communities.api-error.limit-reached_one` | You've reached the limit of {{count}} hosted community. | hostedCommunityApi | — | in-pr4 |
| `communities.api-error.limit-reached_other` | You've reached the limit of {{count}} hosted communities. | hostedCommunityApi | — | in-pr4 |
| `communities.api-error.missing-mapping` | Connect your Buzz identity before creating a community. | hostedCommunityApi | — | in-pr4 |
| `communities.api-error.not-owner` | Only the community owner can do that. | hostedCommunityApi | — | in-pr4 |
| `communities.api-error.pubkey-already-bound` | This Buzz identity is connected to another Builderlab account. | hostedCommunityApi | — | in-pr4 |
| `communities.api-error.relay-unavailable` | Community provisioning is temporarily unavailable. | hostedCommunityApi | — | in-pr4 |
| `communities.api-error.transferee-not-registered` | That person needs a connected Buzz identity before you can transfer ownership to them. | hostedCommunityApi | — | in-pr4 |
| `communities.api-error.with-correlation` | {{message}} Correlation ID: {{correlationId}} | hostedCommunityApi | — | in-pr4 |
| `communities.apply-error.retry` | Retry | CommunityApplyErrorScreen | — | in-pr4 |
| `communities.apply-error.title` | Community connection failed | CommunityApplyErrorScreen | — | in-pr4 |
| `communities.change.description` | Update your community name or relay URL. | CommunityChangeOverlay | — | in-pr4 |
| `communities.change.duplicate-relay` | Another community already uses this relay URL. | CommunityChangeOverlay | — | in-pr4 |
| `communities.change.not-found` | Community not found. | CommunityChangeOverlay | — | in-pr4 |
| `communities.change.save` | Save changes | CommunityChangeOverlay | — | in-pr4 |
| `communities.change.title` | Change community | CommunityApplyErrorScreen, CommunityChangeOverlay | — | in-pr4 |
| `communities.connection.connected` | Connected | CommunitySwitcher | — | in-pr4 |
| `communities.connection.connecting` | Connecting… | CommunitySwitcher | — | in-pr4 |
| `communities.connection.disconnected` | Disconnected from relay | CommunitySwitcher | — | in-pr4 |
| `communities.connection.idle` | Not connected | CommunitySwitcher | — | in-pr4 |
| `communities.connection.reconnecting` | Reconnecting to relay… | CommunitySwitcher | — | in-pr4 |
| `communities.connection.stalled` | Connection lost — relay is not responding | CommunitySwitcher | — | in-pr4 |
| `communities.edit.cancel` | Cancel | EditCommunityDialog | — | in-pr4 |
| `communities.edit.description` | Update this community's name or relay URL. | EditCommunityDialog | — | in-pr4 |
| `communities.edit.icon-heading` | Community icon | EditCommunityDialog | — | in-pr4 |
| `communities.edit.icon-hint` | Shown in the community rail and switcher. | EditCommunityDialog | — | in-pr4 |
| `communities.edit.name-label` | Name | EditCommunityDialog | — | in-pr4 |
| `communities.edit.name-placeholder` | My Community | EditCommunityDialog | — | in-pr4 |
| `communities.edit.optional-suffix` | (optional) | EditCommunityDialog | — | in-pr4 |
| `communities.edit.relay-label` | Relay URL | EditCommunityDialog | — | in-pr4 |
| `communities.edit.repos-dir-help-after` |  directory at an existing folder so agents work in your local checkouts. Leave blank to use the default location. | EditCommunityDialog | — | in-pr4 |
| `communities.edit.repos-dir-help-before` | Point the agent's  | EditCommunityDialog | — | in-pr4 |
| `communities.edit.repos-dir-label` | Repos Directory | EditCommunityDialog | — | in-pr4 |
| `communities.edit.save` | Save Changes | EditCommunityDialog | — | in-pr4 |
| `communities.edit.title` | Edit Community | EditCommunityDialog | — | in-pr4 |
| `communities.edit.token-label` | API Token | EditCommunityDialog | — | in-pr4 |
| `communities.editform.cancel` | Cancel | CommunityEditForm | — | in-pr4 |
| `communities.editform.checking-aria` | Checking relay | CommunityEditForm | — | in-pr4 |
| `communities.editform.error-age` | Confirm that you are at least 18 years old. | CommunityEditForm | — | in-pr4 |
| `communities.editform.error-agreement` | Agree to the Terms of Service and Privacy Policy. | CommunityEditForm | — | in-pr4 |
| `communities.editform.error-name-required` | Please enter a community name. | CommunityEditForm | — | in-pr4 |
| `communities.editform.error-url-invalid` | Enter a valid ws:// or wss:// relay URL. | CommunityEditForm | — | in-pr4 |
| `communities.editform.name-label` | Community name | CommunityEditForm, HostedCommunityOnboarding | — | in-pr4 |
| `communities.editform.name-placeholder` | Design team | CommunityEditForm | — | in-pr4 |
| `communities.editform.saving-aria` | Saving | CommunityEditForm | — | in-pr4 |
| `communities.editform.url-label` | Community URL | CommunityEditForm | — | in-pr4 |
| `communities.editform.use-anyway` | Use anyway | CommunityEditForm | — | in-pr4 |
| `communities.editform.warning-relay-unreachable` | Can't reach this relay — check the URL | CommunityEditForm | — | in-pr4 |
| `communities.hosted.account-row` | Account: {{value}} | HostedCommunityCreateFlow, HostedCommunityOnboarding | — | in-pr4 |
| `communities.hosted.add-new` | + Add new | HostedCommunityOnboarding | — | in-pr4 |
| `communities.hosted.address-available` | That address is available. | HostedCommunityCreateFlow, HostedCommunityOnboarding | — | in-pr4 |
| `communities.hosted.address-label` | Community address | HostedCommunityCreateFlow | — | in-pr4 |
| `communities.hosted.address-locked` | You can’t change this address after creating the community. | HostedCommunityCreateFlow | — | in-pr4 |
| `communities.hosted.address-rules` | Use lowercase letters, numbers, and single hyphens. | HostedCommunityCreateFlow, HostedCommunityOnboarding | — | in-pr4 |
| `communities.hosted.address-status` | Community address status | HostedCommunityOnboarding | — | in-pr4 |
| `communities.hosted.address-taken` | That Buzz address is already taken. | HostedCommunityCreateFlow, HostedCommunityOnboarding, hostedCommunityApi | — | in-pr4 |
| `communities.hosted.address-taken-short` | That address is already taken. | HostedCommunityCreateFlow, HostedCommunityOnboarding | — | in-pr4 |
| `communities.hosted.back` | Back | HostedCommunityOnboarding | — | in-pr4 |
| `communities.hosted.checking-availability` | Checking availability… | HostedCommunityCreateFlow, HostedCommunityOnboarding | — | in-pr4 |
| `communities.hosted.checking-sign-in` | Checking sign-in | HostedCommunityCreateFlow, HostedCommunityOnboarding | — | in-pr4 |
| `communities.hosted.choose-community` | Choose a community | HostedCommunityOnboarding | — | in-pr4 |
| `communities.hosted.choose-community-body` | Connect one you own, or start something new. | HostedCommunityOnboarding | — | in-pr4 |
| `communities.hosted.claim-address-body` | Claim a Buzz address to get started. | HostedCommunityOnboarding | — | in-pr4 |
| `communities.hosted.connect` | Connect | HostedCommunityOnboarding | — | in-pr4 |
| `communities.hosted.connect-and-continue` | Connect and continue | HostedCommunityCreateFlow, HostedCommunityOnboarding | — | in-pr4 |
| `communities.hosted.connect-busy` | {{prefix}}onboarding is already in progress for another community. Go back and finish or restart that connection, then connect this community from your owned communities list. | HostedCommunityOnboarding | — | in-pr4 |
| `communities.hosted.connect-device-identity-error` | Could not connect this device's Buzz identity. | HostedCommunityCreateFlow, HostedCommunityOnboarding | — | in-pr4 |
| `communities.hosted.connect-identity-body` | Connect this device’s Buzz identity to your Builderlab account. Your private key stays on this device. | HostedCommunityCreateFlow | — | in-pr4 |
| `communities.hosted.connect-identity-error` | Could not connect the Buzz identity. | HostedCommunityCreateFlow, HostedCommunityOnboarding | — | in-pr4 |
| `communities.hosted.connect-no-relay` | {{prefix}}Builderlab did not return its relay address. Try connecting it again, or contact support if it does not appear in your communities. | HostedCommunityOnboarding | — | in-pr4 |
| `communities.hosted.connecting-community` | Connecting community… | HostedCommunityOnboarding | — | in-pr4 |
| `communities.hosted.connecting-identity` | Connecting identity… | HostedCommunityCreateFlow, HostedCommunityOnboarding | — | in-pr4 |
| `communities.hosted.continue-to-builderlab` | Continue to Builderlab | HostedCommunityCreateFlow | — | in-pr4 |
| `communities.hosted.create-community` | Create community | HostedCommunityCreateFlow | — | in-pr4 |
| `communities.hosted.create-community-heading` | Create a community | HostedCommunityOnboarding | — | in-pr4 |
| `communities.hosted.create-error` | Could not create the community. | HostedCommunityCreateFlow, HostedCommunityOnboarding | — | in-pr4 |
| `communities.hosted.create-new-question` | Want to create a new community? | HostedCommunityOnboarding | — | in-pr4 |
| `communities.hosted.created-no-url` | The community was created, but Builderlab did not return its community URL. Try connecting it again from settings. | HostedCommunityCreateFlow | — | in-pr4 |
| `communities.hosted.created-prefix` | The community was created, but  | HostedCommunityOnboarding | — | in-pr4 |
| `communities.hosted.creating` | Creating community… | HostedCommunityCreateFlow, HostedCommunityOnboarding | — | in-pr4 |
| `communities.hosted.device-row` | This device: {{value}} | HostedCommunityCreateFlow, HostedCommunityOnboarding | — | in-pr4 |
| `communities.hosted.disconnect-identity-error` | Could not disconnect the account's previous Buzz identity. | HostedCommunityCreateFlow, HostedCommunityOnboarding | — | in-pr4 |
| `communities.hosted.finish-body-after` |  is ready. Connect this device’s Buzz identity to finish setup. Your private key stays on this device. | HostedCommunityOnboarding | — | in-pr4 |
| `communities.hosted.finish-body-before` | Your Builderlab account | HostedCommunityOnboarding | — | in-pr4 |
| `communities.hosted.finish-title` | Finish connecting Buzz | HostedCommunityOnboarding | — | in-pr4 |
| `communities.hosted.hosted-community` | Hosted community | HostedCommunityOnboarding | — | in-pr4 |
| `communities.hosted.identity-mismatch-long` | This device's Buzz identity belongs to a different Builderlab account and can't be moved from here. Sign out, then sign in with the account that already owns this identity. | HostedCommunityOnboarding | — | in-pr4 |
| `communities.hosted.identity-mismatch-short` | This device's Buzz identity belongs to a different Builderlab account. Sign in with the account that already owns this identity. | HostedCommunityCreateFlow | — | in-pr4 |
| `communities.hosted.limit-reached_one` | You’ve reached the limit of {{count}} hosted community. | HostedCommunityCreateFlow, HostedCommunityOnboarding | — | in-pr4 |
| `communities.hosted.limit-reached_other` | You’ve reached the limit of {{count}} hosted communities. | HostedCommunityCreateFlow, HostedCommunityOnboarding | — | in-pr4 |
| `communities.hosted.mismatch-body` | This Builderlab account uses a different Buzz identity. Switch it to this device, or sign in with another account. | HostedCommunityCreateFlow | — | in-pr4 |
| `communities.hosted.mismatch-title` | This account uses a different Buzz identity | HostedCommunityOnboarding | — | in-pr4 |
| `communities.hosted.modal-mismatch-body` | This account is connected to another Buzz identity. Reconnect this device, or sign out to use a different email. | HostedCommunityOnboarding | — | in-pr4 |
| `communities.hosted.name-here-placeholder` | Community name here | HostedCommunityOnboarding | — | in-pr4 |
| `communities.hosted.next` | Next | HostedCommunityOnboarding | — | in-pr4 |
| `communities.hosted.open-source-note` | Buzz is open source. Builderlab hosts the relay for this account. | HostedCommunityOnboarding | — | in-pr4 |
| `communities.hosted.set-name-label` | Set new community name | HostedCommunityOnboarding | — | in-pr4 |
| `communities.hosted.setup-body` | Sign in to connect a community you already own or create a new one. We’ll open Builderlab in your browser, then bring you back to Buzz. | HostedCommunityOnboarding | — | in-pr4 |
| `communities.hosted.setup-title` | Set up your community | HostedCommunityOnboarding | — | in-pr4 |
| `communities.hosted.sign-in-body` | Sign in with Builderlab to create and host a community. Buzz will open your browser, then bring you back here. | HostedCommunityCreateFlow | — | in-pr4 |
| `communities.hosted.sign-in-continue` | Sign in to continue | HostedCommunityOnboarding | — | in-pr4 |
| `communities.hosted.sign-in-different-email` | Sign in with a different email | HostedCommunityOnboarding | — | in-pr4 |
| `communities.hosted.signing-in` | Signing in… | HostedCommunityCreateFlow, HostedCommunityOnboarding | — | in-pr4 |
| `communities.hosted.signing-out` | Signing out… | HostedCommunityCreateFlow, HostedCommunityOnboarding | — | in-pr4 |
| `communities.hosted.switching-identity` | Switching identity… | HostedCommunityCreateFlow, HostedCommunityOnboarding | — | in-pr4 |
| `communities.hosted.use-different-account` | Use a different account | HostedCommunityCreateFlow | — | in-pr4 |
| `communities.hosted.use-this-device` | Use this device | HostedCommunityCreateFlow | — | in-pr4 |
| `communities.hosted.use-this-device-identity` | Use this device's identity | HostedCommunityOnboarding | — | in-pr4 |
| `communities.hosted.waiting-browser` | Waiting for your browser… | HostedCommunityOnboarding | — | in-pr4 |
| `communities.hosted.your-communities` | Your communities | HostedCommunityOnboarding | — | in-pr4 |
| `communities.icon.fallback-label` | Community | CommunityIconSettingsCard | — | in-pr4 |
| `communities.icon.update-failed` | Couldn’t update the community icon. | CommunityIconSettingsCard | — | in-pr4 |
| `communities.nest.migrated-description` | You can delete it to reclaim disk space. | useNestNotifications | — | in-pr4 |
| `communities.nest.migrated-title` | Migrated notes from ~/.sprout | useNestNotifications | — | in-pr4 |
| `communities.nest.repos-dir-error` | Repos directory not applied | useNestNotifications | — | in-pr4 |
| `communities.switcher.actions-aria` | Community actions | CommunitySwitcher | — | in-pr4 |
| `communities.switcher.add` | Add a community | CommunitySwitcher | — | in-pr4 |
| `communities.switcher.copy-url` | Copy community URL | CommunitySwitcher | — | in-pr4 |
| `communities.switcher.edit-aria` | Edit {{name}} | CommunitySwitcher | — | in-pr4 |
| `communities.switcher.fallback-name` | Community | CommunitySwitcher | — | in-pr4 |
| `communities.switcher.invite` | Invite to community | CommunitySwitcher | — | in-pr4 |
| `communities.switcher.leave` | Leave community | CommunitySwitcher | — | in-pr4 |
| `communities.switcher.leave-error` | Couldn't leave the community. Try again. | CommunitySwitcher | — | in-pr4 |
| `communities.switcher.leaving` | Leaving… | CommunitySwitcher | — | in-pr4 |
| `communities.switcher.no-community` | No community | CommunitySwitcher | — | in-pr4 |
| `communities.switcher.removed-description` | You were no longer a member, so Buzz removed the community from this device. | CommunitySwitcher | — | in-pr4 |
| `communities.switcher.removed-toast` | Community removed | CommunitySwitcher | — | in-pr4 |
| `communities.switcher.settings` | Community settings | CommunitySwitcher | — | in-pr4 |
| `communities.switcher.switch` | Switch community | CommunitySwitcher | — | in-pr4 |
| `communities.welcome.copied` | Copied | WelcomeSetup | — | in-pr4 |
| `communities.welcome.copy` | Copy | WelcomeSetup | — | in-pr4 |
| `communities.welcome.copy-npub-aria` | Copy public ID | WelcomeSetup | — | in-pr4 |
| `communities.welcome.create-community` | Create a community | WelcomeSetup | — | in-pr4 |
| `communities.welcome.have-community` | I already have a community | WelcomeSetup | — | in-pr4 |
| `communities.welcome.invite-placeholder` | Invite link or community URL | WelcomeSetup | — | in-pr4 |
| `communities.welcome.join-community` | Join a community | WelcomeSetup | — | in-pr4 |
| `communities.welcome.join-description` | Enter the invite link or community URL you received. | WelcomeSetup | — | in-pr4 |
| `communities.welcome.join-or-create-description` | Join with an invite, create your own community, or reconnect one you already have. | WelcomeSetup | — | in-pr4 |
| `communities.welcome.join-or-create-title` | Join or create a community | WelcomeSetup | — | in-pr4 |
| `communities.welcome.loading` | Loading… | WelcomeSetup | — | in-pr4 |
| `communities.welcome.member-description` | Enter the community URL or an invite link. Your role will be restored when you connect. | WelcomeSetup | — | in-pr4 |
| `communities.welcome.member-or-admin` | I’m a member or admin | WelcomeSetup | — | in-pr4 |
| `communities.welcome.npub-error` | Could not load your public key. | WelcomeSetup | — | in-pr4 |
| `communities.welcome.own-community` | I own the community | WelcomeSetup | — | in-pr4 |
| `communities.welcome.private-body` | Some communities need the owner to add you before you can join. Copy your public ID and send it to the community owner. | WelcomeSetup | — | in-pr4 |
| `communities.welcome.private-heading` | Joining a private community? | WelcomeSetup | — | in-pr4 |
| `communities.welcome.reconnect-description` | Tell us your role so we can find the fastest way back in. | WelcomeSetup | — | in-pr4 |
| `communities.welcome.reconnect-title` | Reconnect to your community | WelcomeSetup | — | in-pr4 |

| `shared.linkPreview.appearance` | Appearance | link-preview-controls | — | in-pr4 |
| `shared.linkPreview.appearance-hint` | You can always modify this and other settings in Appearance. | link-preview-controls | — | in-pr4 |
| `shared.linkPreview.controls-aria` | Link display settings | link-preview-controls | — | in-pr4 |
| `shared.linkPreview.display` | Display | link-preview-controls | — | in-pr4 |
| `shared.linkPreview.expand.show-less` | Show less | rich-link-preview-attachment | — | in-pr4 |
| `shared.linkPreview.expand.show-more` | Show more | rich-link-preview-attachment | — | in-pr4 |
| `shared.linkPreview.image.alt` | Preview from {{domain}} | compact-link-preview-attachment, rich-link-preview-attachment | — | in-pr4 |
| `shared.linkPreview.open-aria` | Open {{provider}} {{type}}: {{title}} | compact-link-preview-attachment, rich-link-preview-attachment | — | in-pr4 |
| `shared.linkPreview.remove` | Remove preview | link-preview-controls | — | in-pr4 |
| `shared.linkPreview.remove-dialog.cancel` | Cancel | link-preview-list | — | in-pr4 |
| `shared.linkPreview.remove-dialog.confirm_one` | Remove preview | link-preview-list | — | in-pr4 |
| `shared.linkPreview.remove-dialog.confirm_other` | Remove previews | link-preview-list | — | in-pr4 |
| `shared.linkPreview.remove-dialog.description_one` | This removes the preview for everyone. | link-preview-list | — | in-pr4 |
| `shared.linkPreview.remove-dialog.description_other` | This removes the previews for everyone. | link-preview-list | — | in-pr4 |
| `shared.linkPreview.remove-dialog.title_one` | Remove preview? | link-preview-list | — | in-pr4 |
| `shared.linkPreview.remove-dialog.title_other` | Remove previews? | link-preview-list | — | in-pr4 |
| `shared.linkPreview.style-changed` | Link previews set to {{style}}. | link-preview-controls | — | in-pr4 |
| `shared.linkPreview.style.compact` | Compact | link-preview-controls | — | in-pr4 |
| `shared.linkPreview.style.rich` | Rich | link-preview-controls | — | in-pr4 |
| `shared.markdown.channel.activity-days` | Active {{count}}d ago | ChannelDeepLink | — | in-pr4 |
| `shared.markdown.channel.activity-hours` | Active {{count}}h ago | ChannelDeepLink | — | in-pr4 |
| `shared.markdown.channel.activity-just-now` | Active just now | ChannelDeepLink | — | in-pr4 |
| `shared.markdown.channel.activity-minutes` | Active {{count}}m ago | ChannelDeepLink | — | in-pr4 |
| `shared.markdown.channel.activity-weeks` | Active {{count}}w ago | ChannelDeepLink | — | in-pr4 |
| `shared.markdown.channel.archived` | Archived | ChannelDeepLink | — | in-pr4 |
| `shared.markdown.channel.aria-name` | Channel {{name}} | ChannelDeepLink | — | in-pr4 |
| `shared.markdown.channel.aria-open-channel` | Open channel: {{label}} | ChannelDeepLink | — | in-pr4 |
| `shared.markdown.channel.aria-open-message` | Open message: {{label}} | ChannelDeepLink | — | in-pr4 |
| `shared.markdown.channel.forum` | Forum | ChannelDeepLink | — | in-pr4 |
| `shared.markdown.channel.private` | Private channel | ChannelDeepLink | — | in-pr4 |
| `shared.markdown.channel.public` | Public channel | ChannelDeepLink | — | in-pr4 |
| `shared.markdown.code.copied` | Copied code to clipboard | CodeBlock | — | in-pr4 |
| `shared.markdown.code.copy-aria` | Copy code block | CodeBlock | — | in-pr4 |
| `shared.markdown.code.copy-failed` | Failed to copy code | CodeBlock | — | in-pr4 |
| `shared.markdown.code.copy-tooltip` | Copy code | CodeBlock | — | in-pr4 |
| `shared.markdown.emoji.custom` | Custom emoji | InlineEmojiPopover | — | in-pr4 |
| `shared.markdown.entity.aria-open-commit` | Open commit {{hash}} in repository {{name}} | entityLinks | — | in-pr4 |
| `shared.markdown.entity.footer-issue` | Issue · {{repository}} | entityLinks | — | in-pr4 |
| `shared.markdown.entity.footer-pr` | Pull request · {{repository}} | entityLinks | — | in-pr4 |
| `shared.markdown.entity.footer-project` | Project | entityLinks | — | in-pr4 |
| `shared.markdown.entity.footer-repo` | Repository | entityLinks | — | in-pr4 |
| `shared.markdown.gallery.position-aria` | Image {{current}} of {{total}} | ImageGalleryStatus | — | in-pr4 |
| `shared.markdown.image.copied` | Copied to clipboard | imageActions | — | in-pr4 |
| `shared.markdown.image.copy-failed` | Copy failed | imageActions | — | in-pr4 |
| `shared.markdown.image.download-failed` | Download failed | FileCard, imageActions | — | in-pr4 |
| `shared.markdown.link-pill.age-days` | {{count}}d ago | MessageLinkPill | — | in-pr4 |
| `shared.markdown.link-pill.age-hours` | {{count}}h ago | MessageLinkPill | — | in-pr4 |
| `shared.markdown.link-pill.age-just-now` | just now | MessageLinkPill | — | in-pr4 |
| `shared.markdown.link-pill.age-minutes` | {{count}}m ago | MessageLinkPill | — | in-pr4 |
| `shared.markdown.link-pill.age-weeks` | {{count}}w ago | MessageLinkPill | — | in-pr4 |
| `shared.markdown.link-pill.aria-in-channel` | Message in channel {{name}} | MessageLinkPill | — | in-pr4 |
| `shared.markdown.link-pill.aria-open-channel-deleted` | Open channel {{name}}; linked message was deleted | MessageLinkPill | — | in-pr4 |
| `shared.markdown.link-pill.aria-open-message-in-channel` | Open message in channel {{name}} | MessageLinkPill | — | in-pr4 |
| `shared.markdown.link-pill.aria-open-thread-deleted` | Open thread in channel {{name}}; linked message was deleted | MessageLinkPill | — | in-pr4 |
| `shared.markdown.link-pill.aria-open-thread-in` | Open thread in {{name}} | MessageLinkPill | — | in-pr4 |
| `shared.markdown.link-pill.deleted` | Message deleted | MessageLinkPill | — | in-pr4 |
| `shared.markdown.link-pill.dm-with` | Direct message with {{destination}} | MessageLinkPill | — | in-pr4 |
| `shared.markdown.link-pill.forum-post-in` | Forum post in {{destination}} | MessageLinkPill | — | in-pr4 |
| `shared.markdown.link-pill.thread-in` | Thread in {{destination}} | MessageLinkPill | — | in-pr4 |
| `shared.markdown.link-pill.unavailable` | Message unavailable | MessageLinkPill | — | in-pr4 |
| `shared.markdown.link.copied` | Link copied to clipboard | BuzzLinkChip, ExternalLinkAnchor | — | in-pr4 |
| `shared.markdown.link.copy` | Copy link | BuzzLinkChip, ExternalLinkAnchor | — | in-pr4 |
| `shared.markdown.link.open` | Open link | BuzzLinkChip, ExternalLinkAnchor | — | in-pr4 |
| `shared.markdown.link.open-failed` | Failed to open link | ExternalLinkAnchor | — | in-pr4 |
| `shared.markdown.snapshot.actions-aria` | Actions for {{name}} | AgentSnapshotCard | — | in-pr4 |
| `shared.markdown.snapshot.add-agent` | Add agent | AgentSnapshotCard | — | in-pr4 |
| `shared.markdown.snapshot.add-team` | Add team | AgentSnapshotCard | — | in-pr4 |
| `shared.markdown.snapshot.download` | Download | AgentSnapshotCard | — | in-pr4 |
| `shared.markdown.snapshot.download-aria` | Download {{name}} | AgentSnapshotCard | — | in-pr4 |
| `shared.markdown.snapshot.load-failed-agent` | Couldn’t load this agent. Try again. | AgentSnapshotCard | — | in-pr4 |
| `shared.markdown.snapshot.load-failed-team` | Couldn’t load this team. Try again. | AgentSnapshotCard | — | in-pr4 |
| `shared.markdown.snapshot.loading` | Loading… | AgentSnapshotCard | — | in-pr4 |
| `shared.markdown.snapshot.shared-by` | Shared by {{sharer}} | AgentSnapshotCard | — | in-pr4 |
| `shared.markdown.spoiler.hide` | Hide spoiler | SpoilerInline | — | in-pr4 |
| `shared.markdown.spoiler.reveal` | Reveal spoiler | SpoilerInline | — | in-pr4 |
| `shared.markdown.task.completed-aria` | Completed task | MarkdownInput | — | in-pr4 |
| `shared.markdown.task.incomplete-aria` | Incomplete task | MarkdownInput | — | in-pr4 |
| `shared.markdown.video.title` | Video | MarkdownVideoPlayer | — | in-pr4 |
| `shared.markdown.zoom.image-aria` | Zoom image: {{alt}} | LinkPreviewImageLightbox | — | in-pr4 |
| `shared.markdown.zoom.in-aria` | Zoom in | ImageLightboxZoomControls | — | in-pr4 |
| `shared.markdown.zoom.out-aria` | Zoom out | ImageLightboxZoomControls | — | in-pr4 |
| `shared.markdown.zoom.slider-aria` | Image zoom | ImageLightboxZoomControls | — | in-pr4 |

| `gifs.choose-aria` | Choose {{title}} | KlipyGifPicker | — | in-pr4 |
| `gifs.empty` | No GIFs found. | KlipyGifPicker | — | in-pr4 |
| `gifs.error.capabilities-failed` | Could not read relay capabilities ({{status}}) | relay | — | in-pr4 |
| `gifs.error.relay-membership-required` | Join this community to search GIFs. | relay | — | in-pr4 |
| `gifs.error.request-failed` | GIF request failed ({{status}}) | relay | — | in-pr4 |
| `gifs.error.search-failed` | GIF search failed | relay | — | in-pr4 |
| `gifs.loading` | Loading GIFs | KlipyGifPicker | — | in-pr4 |
| `gifs.powered-by` | Powered by KLIPY | KlipyGifPicker | — | in-pr4 |
| `gifs.retry` | Try again | KlipyGifPicker | — | in-pr4 |
| `gifs.search.aria` | Search KLIPY | KlipyGifPicker | — | in-pr4 |
| `gifs.search.placeholder` | Search KLIPY | KlipyGifPicker | — | in-pr4 |
| `terminal.aria` | Buzz Term | TerminalSubstrate | — | in-pr4 |
| `terminal.close-tab-aria` | Close {{title}} | TerminalSubstrate | — | in-pr4 |
| `terminal.closing` | Closing… | TerminalSubstrate | — | in-pr4 |
| `terminal.hide-aria` | Hide Buzz Term | TerminalSubstrate | — | in-pr4 |
| `terminal.input-aria` | Terminal input | TerminalSubstrate | — | in-pr4 |
| `terminal.maximize-aria` | Maximize Buzz Term | TerminalSubstrate | — | in-pr4 |
| `terminal.mode-buzz` | Buzz mode | TerminalSubstrate | — | in-pr4 |
| `terminal.mode-buzz-term` | Buzz Term mode | TerminalSubstrate | — | in-pr4 |
| `terminal.new-tab-aria` | New Buzz Term tab | TerminalSubstrate | — | in-pr4 |
| `terminal.resize-aria` | Resize Buzz Term | TerminalSubstrate | — | in-pr4 |
| `terminal.restore-aria` | Restore Buzz Term | TerminalSubstrate | — | in-pr4 |
| `terminal.tab-aria` | Terminal {{number}} | TerminalSubstrate | — | in-pr4 |
| `terminal.tab-aria-closing` | Terminal {{number}}, closing | TerminalSubstrate | — | in-pr4 |
| `terminal.tab-aria-titled` | Terminal {{number}}, {{title}} | TerminalSubstrate | — | in-pr4 |

| `identity-archive.archive-failed` | Archive failed: {{error}} | hooks | — | in-pr4 |
| `identity-archive.archived` | Archived on this relay | hooks | — | in-pr4 |
| `identity-archive.unarchive-failed` | Unarchive failed: {{error}} | hooks | — | in-pr4 |
| `identity-archive.unarchived` | Unarchived on this relay | hooks | — | in-pr4 |
| `profile.agent-actions.action-failed` | Agent action failed. | useAgentLifecycleActions | — | in-pr4 |
| `profile.agent-actions.archive-identity` | Archive identity | UserProfileAgentActions | — | in-pr4 |
| `profile.agent-actions.auto-start` | Auto-start | UserProfileAgentActions | — | in-pr4 |
| `profile.agent-actions.deploying` | Deploying {{name}}. | useAgentLifecycleActions | — | in-pr4 |
| `profile.agent-actions.duplicate` | Duplicate | UserProfileAgentActions | — | in-pr4 |
| `profile.agent-actions.export` | Export | UserProfileAgentActions | — | in-pr4 |
| `profile.agent-actions.open-settings-aria` | Open profile settings | UserProfileAgentActions | — | in-pr4 |
| `profile.agent-actions.restart-failed` | Agent restart failed. | useAgentLifecycleActions | — | in-pr4 |
| `profile.agent-actions.restarted` | Restarted {{name}}. | useAgentLifecycleActions | — | in-pr4 |
| `profile.agent-actions.started` | Started {{name}}. | useAgentLifecycleActions | — | in-pr4 |
| `profile.agent-actions.stopped` | Stopped {{name}}. | useAgentLifecycleActions | — | in-pr4 |
| `profile.agent-actions.unarchive-identity` | Unarchive identity | UserProfileAgentActions | — | in-pr4 |
| `profile.agent-management.archive` | Archive agent | UserProfileAgentActions, UserProfileAgentManagementRows | — | in-pr4 |
| `profile.agent-management.archiving` | Archiving… | UserProfileAgentActions, UserProfileAgentManagementRows | — | in-pr4 |
| `profile.agent-management.cancel` | Cancel | UserProfileAgentManagementRows | — | in-pr4 |
| `profile.agent-management.create-card` | Create trading card | UserProfileAgentManagementRows | — | in-pr4 |
| `profile.agent-management.delete-agent` | Delete agent | UserProfileAgentManagementRows | — | in-pr4 |
| `profile.agent-management.delete-archive-hint` | Archive this agent if you want to hide it instead of removing it. | UserProfileAgentManagementRows | — | in-pr4 |
| `profile.agent-management.delete-desc-local` | Deleting this agent stops and removes the agent from this community. | UserProfileAgentManagementRows | — | in-pr4 |
| `profile.agent-management.delete-desc-provider` | Deleting removes this agent’s local management record, not its remote deployment. | UserProfileAgentManagementRows | — | in-pr4 |
| `profile.agent-management.delete-list-channels` | Removes the agent from every channel it belongs to | UserProfileAgentManagementRows | — | in-pr4 |
| `profile.agent-management.delete-list-local` | Stops any local agent process before deleting the record | UserProfileAgentManagementRows | — | in-pr4 |
| `profile.agent-management.delete-list-provider` | Unless the agent is known to be Offline, Buzz first requests shutdown through a channel when available. A failed request cancels deletion. The remote process may still be running even after a successful request. | UserProfileAgentManagementRows | — | in-pr4 |
| `profile.agent-management.delete-list-record` | Removes the local management record and saved agent key | UserProfileAgentManagementRows | — | in-pr4 |
| `profile.agent-management.delete-list-relay` | Archives the agent's identity on the relay so it no longer appears in member lists or mention suggestions | UserProfileAgentManagementRows | — | in-pr4 |
| `profile.agent-management.delete-title` | Delete this agent? | UserProfileAgentManagementRows | — | in-pr4 |
| `profile.agent-management.deleting` | Deleting… | UserProfileAgentManagementRows | — | in-pr4 |
| `profile.agent-management.duplicate` | Duplicate agent | UserProfileAgentManagementRows | — | in-pr4 |
| `profile.agent-management.export` | Export agent | UserProfileAgentManagementRows | — | in-pr4 |
| `profile.agent-management.unarchive` | Unarchive agent | UserProfileAgentActions, UserProfileAgentManagementRows | — | in-pr4 |
| `profile.agent-management.unarchiving` | Unarchiving… | UserProfileAgentActions, UserProfileAgentManagementRows | — | in-pr4 |
| `profile.archive-confirm.archive` | Archive | ArchiveConfirmDialog | — | in-pr4 |
| `profile.archive-confirm.archiving` | Archiving… | ArchiveConfirmDialog | — | in-pr4 |
| `profile.archive-confirm.bot-note` | You can also delete this agent from the profile settings menu if you want to remove the agent instead of hiding it. | ArchiveConfirmDialog | — | in-pr4 |
| `profile.archive-confirm.cancel` | Cancel | ArchiveConfirmDialog | — | in-pr4 |
| `profile.archive-confirm.hides` | Archiving hides {{subject}} from the space. | ArchiveConfirmDialog | — | in-pr4 |
| `profile.archive-confirm.list-restore` | You can unarchive them at any time to restore them | ArchiveConfirmDialog | — | in-pr4 |
| `profile.archive-confirm.list-scope-after` |  — not their account anywhere else | ArchiveConfirmDialog | — | in-pr4 |
| `profile.archive-confirm.list-scope-before` | This only affects  | ArchiveConfirmDialog | — | in-pr4 |
| `profile.archive-confirm.list-scope-emphasis` | this space | ArchiveConfirmDialog | — | in-pr4 |
| `profile.archive-confirm.list-search` | They won't appear in search, autocomplete, or when adding members | ArchiveConfirmDialog | — | in-pr4 |
| `profile.archive-confirm.subject-agent` | this agent | ArchiveConfirmDialog | — | in-pr4 |
| `profile.archive-confirm.subject-person` | this person | ArchiveConfirmDialog | — | in-pr4 |
| `profile.archive-confirm.title-agent` | Archive this agent? | ArchiveConfirmDialog | — | in-pr4 |
| `profile.archive-confirm.title-identity` | Archive this identity? | ArchiveConfirmDialog | — | in-pr4 |
| `profile.avatar-mode.animated` | Animated | ProfileAvatarModeTabs | — | in-pr4 |
| `profile.avatar-mode.emoji` | Emoji | ProfileAvatarModeTabs | — | in-pr4 |
| `profile.avatar-mode.image` | Image | ProfileAvatarModeTabs | — | in-pr4 |
| `profile.avatar-mode.type-aria` | Avatar type | ProfileAvatarModeTabs | — | in-pr4 |
| `profile.details.instructions` | Agent instructions | UserProfilePanelAgentDetails | — | in-pr4 |
| `profile.details.no-instructions` | No instruction set. | UserProfilePanelAgentDetails | — | in-pr4 |
| `profile.fields.acp-command` | ACP command | UserProfilePanelFields | — | in-pr4 |
| `profile.fields.agent-profile` | Agent profile | UserProfilePanelFields | — | in-pr4 |
| `profile.fields.agent-type` | Agent type | UserProfilePanelFields | — | in-pr4 |
| `profile.fields.backend` | Backend | UserProfilePanelFields | — | in-pr4 |
| `profile.fields.capabilities` | Capabilities | UserProfilePanelFields | — | in-pr4 |
| `profile.fields.copy-aria` | Copy {{name}} | UserProfilePanelFields | — | in-pr4 |
| `profile.fields.last-error` | Last error | UserProfilePanelFields | — | in-pr4 |
| `profile.fields.managed-by` | Managed by | UserProfilePanelFields | — | in-pr4 |
| `profile.fields.mcp-command` | MCP command | UserProfilePanelFields | — | in-pr4 |
| `profile.fields.not-deployed` | Not deployed | UserProfilePanelFields | — | in-pr4 |
| `profile.fields.open-aria` | Open {{name}} | UserProfilePanelFields | — | in-pr4 |
| `profile.fields.owner-verified` | Declared owner verified | UserProfilePanelFields | — | in-pr4 |
| `profile.fields.public-key` | Public key | UserProfilePanelFields | — | in-pr4 |
| `profile.fields.runtime` | Runtime | UserProfilePanelFields | — | in-pr4 |
| `profile.fields.start-on-launch` | Start on launch | UserProfilePanelFields | — | in-pr4 |
| `profile.fields.status` | Status | UserProfilePanelFields | — | in-pr4 |
| `profile.fields.visibility` | Visibility | UserProfilePanelFields | — | in-pr4 |
| `profile.fields.who-can-send-instructions` | Who can send instructions | UserProfilePanelFields | — | in-pr4 |
| `profile.header.back-aria` | Back to profile | UserProfilePanelHeaderContent | — | in-pr4 |
| `profile.header.copy-log` | Copy log | UserProfilePanelHeaderContent | — | in-pr4 |
| `profile.header.edit` | Edit | UserProfilePanelHeaderContent | — | in-pr4 |
| `profile.header.edit-agent-aria` | Edit agent | UserProfilePanelHeaderContent | — | in-pr4 |
| `profile.instances.archived` | Archived | ProfileInstancesSection | — | in-pr4 |
| `profile.instances.count_one` | {{count}} instance | ProfileInstancesSection | — | in-pr4 |
| `profile.instances.count_other` | {{count}} instances | ProfileInstancesSection | — | in-pr4 |
| `profile.instances.current` | Current | ProfileInstancesSection | — | in-pr4 |
| `profile.instances.title` | Instances | ProfileInstancesSection | — | in-pr4 |
| `profile.interaction.dm-failed` | Couldn't open the direct message. | useProfileInteractionActions | — | in-pr4 |
| `profile.interaction.wave-failed` | Couldn't send the wave. | useProfileInteractionActions | — | in-pr4 |
| `profile.panel.view-channels` | Channels | UserProfilePanelUtils | — | in-pr4 |
| `profile.panel.view-configuration` | Runtime | UserProfilePanelUtils | — | in-pr4 |
| `profile.panel.view-harness-log` | Harness log | UserProfilePanelUtils | — | in-pr4 |
| `profile.panel.view-info` | Agent info | UserProfilePanelUtils | — | in-pr4 |
| `profile.panel.view-instructions` | Agent instructions | UserProfilePanelUtils | — | in-pr4 |
| `profile.panel.view-memories` | Memories | UserProfilePanelUtils | — | in-pr4 |
| `profile.panel.view-summary` | Profile | UserProfilePanelUtils | — | in-pr4 |
| `profile.persona.created-and-started` | Created and started {{name}}. | UserProfilePanelPersonaSubmit | — | in-pr4 |
| `profile.persona.created-instance-failed` | {{name}} was created, but the agent instance could not be created: {{error}} | UserProfilePanelPersonaSubmit | — | in-pr4 |
| `profile.persona.created-instance-failed-plain` | {{name}} was created, but the agent instance could not be created. | UserProfilePanelPersonaSubmit | — | in-pr4 |
| `profile.persona.created-not-started` | {{name}} was created, but it did not start: {{error}} | UserProfilePanelPersonaSubmit | — | in-pr4 |
| `profile.persona.created-sync-failed` | {{name}} was created, but profile sync failed: {{error}} | UserProfilePanelPersonaSubmit | — | in-pr4 |
| `profile.persona.runtime-unavailable` | {{label}} is not available. Install it before saving this linked agent. | UserProfilePanelPersonaSubmit | — | in-pr4 |
| `profile.persona.save-failed` | Failed to save agent. | UserProfilePanelPersonaSubmit | — | in-pr4 |
| `profile.persona.sync-failed-updated` | {{name}} was updated, but profile sync failed: {{error}} | UserProfilePanelPersonaSubmit | — | in-pr4 |
| `profile.persona.this-provider` | This provider | UserProfilePanelPersonaSubmit | — | in-pr4 |
| `profile.persona.updated` | Updated {{name}}. | UserProfilePanelPersonaSubmit | — | in-pr4 |
| `profile.primary-actions.follow` | Follow | UserProfilePrimaryActions | — | in-pr4 |
| `profile.primary-actions.follow-failed` | Follow failed: {{error}} | UserProfilePrimaryActions | — | in-pr4 |
| `profile.primary-actions.huddle` | Huddle | UserProfilePrimaryActions | — | in-pr4 |
| `profile.primary-actions.message` | Message | UserProfilePrimaryActions | — | in-pr4 |
| `profile.primary-actions.restart-agent` | Restart agent | UserProfilePrimaryActions | — | in-pr4 |
| `profile.primary-actions.start-agent` | Start agent | UserProfilePrimaryActions | — | in-pr4 |
| `profile.primary-actions.unfollow` | Unfollow | UserProfilePrimaryActions | — | in-pr4 |
| `profile.primary-actions.unfollow-failed` | Unfollow failed: {{error}} | UserProfilePrimaryActions | — | in-pr4 |
| `profile.primary-actions.wave` | Wave | UserProfilePrimaryActions | — | in-pr4 |
| `profile.recipient.remove-aria` | Remove {{name}} | SelectedRecipientChip | — | in-pr4 |
| `profile.recipient.verify-heading` | Verify {{name}} | SelectedRecipientChip | — | in-pr4 |
| `profile.recipient.verify-key-aria` | Verify {{name}} public key | SelectedRecipientChip | — | in-pr4 |
| `profile.snapshot.export-failed` | Failed to export agent snapshot. | UserProfileSnapshotExportDialog | — | in-pr4 |
| `profile.snapshot.exported` | Exported {{name}}. | UserProfileSnapshotExportDialog | — | in-pr4 |

| `shared.ui.carousel.next` | Next slide | carousel | — | in-pr4 |
| `shared.ui.carousel.previous` | Previous slide | carousel | — | in-pr4 |
| `shared.ui.close` | Close | dialog, sheet | — | in-pr4 |
| `shared.ui.config-nudge.adapter-missing` | {{harness}} ACP adapter isn't installed | config-nudge-attachment | — | in-pr4 |
| `shared.ui.config-nudge.adapter-outdated` | {{harness}} ACP adapter is outdated — reinstall required | config-nudge-attachment | — | in-pr4 |
| `shared.ui.config-nudge.cli-missing` | {{harness}} CLI is missing | config-nudge-attachment | — | in-pr4 |
| `shared.ui.config-nudge.cli-not-installed` | {{harness}} isn't installed | config-nudge-attachment | — | in-pr4 |
| `shared.ui.config-nudge.cli-tool-fallback` | the CLI tool | config-nudge-attachment | — | in-pr4 |
| `shared.ui.config-nudge.config-invalid-prefix` | {{file}} is invalid: | config-nudge-attachment | — | in-pr4 |
| `shared.ui.config-nudge.config-invalid-suffix` | — fix the config and restart the agent | config-nudge-attachment | — | in-pr4 |
| `shared.ui.config-nudge.edit-agent-cta` | Edit Agent → | config-nudge-attachment | — | in-pr4 |
| `shared.ui.config-nudge.env-key-prefix` | Set | config-nudge-attachment | — | in-pr4 |
| `shared.ui.config-nudge.env-key-suffix` | in Edit Agent → Environment variables | config-nudge-attachment | — | in-pr4 |
| `shared.ui.config-nudge.field-prefix` | Set the | config-nudge-attachment | — | in-pr4 |
| `shared.ui.config-nudge.field-suffix` | field in Edit Agent dropdowns | config-nudge-attachment | — | in-pr4 |
| `shared.ui.config-nudge.git-bash-required` | Git for Windows is required for buzz-agent shell tools | config-nudge-attachment | — | in-pr4 |
| `shared.ui.config-nudge.missing-binary` | not found in PATH — install it or update PATH, then restart Buzz | config-nudge-attachment | — | in-pr4 |
| `shared.ui.config-nudge.needs-configuration` | {{agent}} needs configuration | config-nudge-attachment | — | in-pr4 |
| `shared.ui.config-nudge.open-agent-runtimes` | Open Agent runtimes → | config-nudge-attachment | — | in-pr4 |
| `shared.ui.config-nudge.open-agent-runtimes-aria` | Open Agent runtimes for {{agent}} | config-nudge-attachment | — | in-pr4 |
| `shared.ui.config-nudge.open-edit-agent-aria` | Open Edit Agent for {{agent}} | config-nudge-attachment | — | in-pr4 |
| `shared.ui.hover-copy.copied` | Copied {{label}} | HoverCopyIndicator | — | in-pr4 |
| `shared.ui.hover-copy.copy-failed` | Couldn't copy {{label}}. | HoverCopyIndicator | — | in-pr4 |
| `shared.ui.image-lightbox.close-aria` | Close lightbox | SimpleImageLightbox | — | in-pr4 |
| `shared.ui.image-lightbox.description` | Full-size image preview. Press Escape or click outside the image to close. | SimpleImageLightbox | — | in-pr4 |
| `shared.ui.pubkey.copied` | {{label}} copied | PubKey | — | in-pr4 |
| `shared.ui.pubkey.copy-aria` | Copy {{label}} | PubKey | — | in-pr4 |
| `shared.ui.pubkey.copy-full-aria` | Copy public key | PubKey | — | in-pr4 |
| `shared.ui.pubkey.show-full-aria` | Show full public key | PubKey | — | in-pr4 |
| `shared.ui.sidebar.description` | Displays the mobile sidebar. | sidebar | — | in-pr4 |
| `shared.ui.sidebar.resize-aria` | Resize sidebar | sidebar | — | in-pr4 |
| `shared.ui.sidebar.resize-title` | Drag to resize sidebar | sidebar | — | in-pr4 |
| `shared.ui.sidebar.title` | Sidebar | sidebar | — | in-pr4 |
| `shared.ui.step-progress.aria` | Step {{current}} of {{total}} | step-progress | — | in-pr4 |
| `shared.ui.user-avatar.alt` | {{name}} avatar | UserAvatar | — | in-pr4 |
| `shared.ui.video-menu.copy-link` | Copy link | useVideoContextMenu | — | in-pr4 |
| `shared.ui.video-menu.download` | Download video | useVideoContextMenu | — | in-pr4 |
| `shared.ui.video-menu.download-failed` | Download failed | useVideoContextMenu | — | in-pr4 |
| `shared.ui.video-menu.link-copied` | Link copied to clipboard | useVideoContextMenu | — | in-pr4 |
| `shared.ui.video-review.jump-to-aria` | Jump to {{timecode}} | VideoReviewTimecodeButton | — | in-pr4 |
| `shared.ui.view-loading.projects` | Loading projects | ViewLoadingFallback | — | in-pr4 |

| `agents.advanced-badge.required` | Required | AdvancedRequiredBadge, EnvVarsEditor | — | in-pr4 |
| `agents.advanced-fields.conversation-context` | Conversation context | PersonaAdvancedFields | — | in-pr4 |
| `agents.advanced-fields.each-thread` | Each thread | PersonaAdvancedFields | — | in-pr4 |
| `agents.advanced-fields.entire-channel` | Entire channel | PersonaAdvancedFields | — | in-pr4 |
| `agents.advanced-fields.name-pool` | Instance name pool | PersonaAdvancedFields | — | in-pr4 |
| `agents.advanced-fields.parallelism` | Parallelism | PersonaAdvancedFields | — | in-pr4 |
| `agents.advanced-fields.thread-separate-desc` | Keeps a separate conversation for each channel thread. Direct messages remain shared. | PersonaAdvancedFields | — | in-pr4 |
| `agents.advanced-fields.thread-shared-desc` | Shares one conversation across every thread in a channel. | PersonaAdvancedFields | — | in-pr4 |
| `agents.ai-config.customize` | Customize for this agent | AgentAiConfigurationMode | — | in-pr4 |
| `agents.ai-config.harness` | Harness | AgentAiConfigurationMode, AgentAiDefaults | — | in-pr4 |
| `agents.ai-config.harness-default` | Harness default | AgentAiConfigurationMode | — | in-pr4 |
| `agents.ai-config.not-configured` | Not configured | AgentAiConfigurationMode, AgentAiDefaults | — | in-pr4 |
| `agents.ai-config.section` | AI configuration | AgentAiConfigurationMode | — | in-pr4 |
| `agents.ai-config.use-agent-defaults` | Use agent defaults | AgentAiConfigurationMode | — | in-pr4 |
| `agents.ai-config.use-harness-defaults` | Use harness defaults | AgentAiConfigurationMode | — | in-pr4 |
| `agents.ai-defaults.edit-global-defaults` | Edit global defaults | AgentAiDefaults | — | in-pr4 |
| `agents.ai-defaults.global-not-set` | Global defaults not set | AgentAiDefaults | — | in-pr4 |
| `agents.ai-defaults.provider` | Provider | AgentAiDefaults, AgentConfigFields | — | in-pr4 |
| `agents.ai-defaults.set` | Set | AgentAiDefaults | — | in-pr4 |
| `agents.api-key.checking` | Checking API key… | PersonaProviderApiKeyField | — | in-pr4 |
| `agents.api-key.hide-aria` | Hide API key | PersonaProviderApiKeyField | — | in-pr4 |
| `agents.api-key.paste-placeholder` | Paste API key… | PersonaProviderApiKeyField | — | in-pr4 |
| `agents.api-key.show-aria` | Show API key | PersonaProviderApiKeyField | — | in-pr4 |
| `agents.card-mint.chip-failed` | Minting {{name}}’s card failed | CardMintComposerChip | — | in-pr4 |
| `agents.card-mint.chip-minting` | Minting {{name}}’s card… (takes a few minutes) | CardMintComposerChip | — | in-pr4 |
| `agents.card-mint.chip-ready` | {{name}}’s card is ready — view it | CardMintComposerChip | — | in-pr4 |
| `agents.card-mint.dismiss` | Dismiss failed mint | CardMintComposerChip | — | in-pr4 |
| `agents.card-mint.failed` | Card mint failed. | cardMintStore | — | in-pr4 |
| `agents.card-mint.invalid-key` | The OpenAI API key is invalid or expired. Open the mint dialog and use "Update API key" to replace it. | cardMintStore | — | in-pr4 |
| `agents.card-mint.key-cue-prefix` | Card-minting key | CardMintKeyCue | — | in-pr4 |
| `agents.card-mint.key-cue-suffix` | is set under Advanced → Environment variables. | CardMintKeyCue | — | in-pr4 |
| `agents.card-mint.mint-failed` | Minting {{name}}'s card failed | cardMintStore | — | in-pr4 |
| `agents.card-mint.ready` | {{name}}'s card is ready | cardMintStore | — | in-pr4 |
| `agents.card-mint.view` | View card | cardMintStore | — | in-pr4 |
| `agents.config-fields.api-key` | API Key | AgentConfigFields, EditAgentProviderModelFields | — | in-pr4 |
| `agents.config-fields.custom-global-provider-id` | Custom global provider ID | AgentConfigFields | — | in-pr4 |
| `agents.config-fields.custom-provider` | Custom provider… | AgentConfigFields | — | in-pr4 |
| `agents.config-fields.custom-provider-id` | Custom provider ID | AgentConfigFields, EditAgentProviderModelFields | — | in-pr4 |
| `agents.config-fields.effort` | Effort | AgentConfigFields | — | in-pr4 |
| `agents.config-fields.environment-variables` | Environment variables | AgentConfigFields | — | in-pr4 |
| `agents.config-fields.select-provider` | Select provider | AgentConfigFields | — | in-pr4 |
| `agents.defaults-dialog.description` | These settings apply to all agents unless you override them. Agent-specific settings always take priority. Changes may restart running agents. | AgentDefaultsDialog | — | in-pr4 |
| `agents.defaults-dialog.discard` | Discard changes | AgentDefaultsDialog | — | in-pr4 |
| `agents.defaults-dialog.discard-description` | Unsaved changes made to agent defaults will be lost. | AgentDefaultsDialog | — | in-pr4 |
| `agents.defaults-dialog.discard-title` | Discard changes to agent defaults? | AgentDefaultsDialog | — | in-pr4 |
| `agents.defaults-dialog.keep-editing` | Keep editing | AgentDefaultsDialog | — | in-pr4 |
| `agents.defaults-editor.load-failed` | Couldn't load agent defaults. Restart the app to try again. | AgentDefaultsEditor | — | in-pr4 |
| `agents.defaults-editor.save` | Save defaults | AgentDefaultsEditor | — | in-pr4 |
| `agents.defaults-editor.saved` | Saved. | AgentDefaultsEditor | — | in-pr4 |
| `agents.defaults-editor.saved-failed_one` | Saved. {{count}} agent couldn't restart — check the Agents page. | AgentDefaultsEditor | — | in-pr4 |
| `agents.defaults-editor.saved-failed_other` | Saved. {{count}} agents couldn't restart — check the Agents page. | AgentDefaultsEditor | — | in-pr4 |
| `agents.defaults-editor.saved-restarted-with-failures_one` | Saved. Restarted {{count}} agent. {{failed}} couldn't restart — check the Agents page. | AgentDefaultsEditor | — | in-pr4 |
| `agents.defaults-editor.saved-restarted-with-failures_other` | Saved. Restarted {{count}} agents. {{failed}} couldn't restart — check the Agents page. | AgentDefaultsEditor | — | in-pr4 |
| `agents.defaults-editor.saved-restarted_one` | Saved. Restarted {{count}} agent. | AgentDefaultsEditor | — | in-pr4 |
| `agents.defaults-editor.saved-restarted_other` | Saved. Restarted {{count}} agents. | AgentDefaultsEditor | — | in-pr4 |
| `agents.definition-footer.catalog-notice` | This agent is in the community catalog. Your changes will be published when you save. | AgentDefinitionDialogFooter | — | in-pr4 |
| `agents.definition-footer.save-and-publish` | Save and publish | AgentDefinitionDialogFooter | — | in-pr4 |
| `agents.definition-footer.uploading` | Uploading... | AgentDefinitionDialogFooter | — | in-pr4 |
| `agents.definition-metadata.built-in-agent` | Built-in agent | AgentDefinitionMetadata | — | in-pr4 |
| `agents.definition-metadata.custom-agent` | Custom agent | AgentDefinitionMetadata | — | in-pr4 |
| `agents.definition-metadata.preferred-model` | Preferred model | AgentDefinitionMetadata | — | in-pr4 |
| `agents.definition-metadata.preferred-provider` | Preferred provider | AgentDefinitionMetadata | — | in-pr4 |
| `agents.definition-metadata.preferred-runtime` | Preferred runtime | AgentDefinitionMetadata | — | in-pr4 |
| `agents.definition-metadata.use-app-default` | Use app default | AgentDefinitionMetadata | — | in-pr4 |
| `agents.edit-provider.llm-provider` | LLM provider | EditAgentProviderModelFields | — | in-pr4 |
| `agents.effort.applies-next-session` | Applied at the next session start. | EffortPickerField | — | in-pr4 |
| `agents.effort.field-label` | Thinking effort | EffortPickerField | — | in-pr4 |
| `agents.env-vars.add` | Add variable | EnvVarsEditor | — | in-pr4 |
| `agents.env-vars.empty` | No variables set. | EnvVarsEditor | — | in-pr4 |
| `agents.env-vars.inherited-from` | Inherited from {{source}} | EnvVarsEditor | — | in-pr4 |
| `agents.env-vars.name-aria` | Variable name | EnvVarsEditor | — | in-pr4 |
| `agents.env-vars.overrides-value` | Overrides {{label}} value | EnvVarsEditor | — | in-pr4 |
| `agents.env-vars.remove-aria` | Remove variable | EnvVarsEditor | — | in-pr4 |
| `agents.env-vars.set-in-goose` | Set in goose config | EnvVarsEditor | — | in-pr4 |
| `agents.env-vars.value-aria` | Value for {{key}} | EnvVarsEditor | — | in-pr4 |
| `agents.env-vars.value-input-aria` | Variable value | EnvVarsEditor | — | in-pr4 |
| `agents.file-read.no-content` | No file content returned. | agentSessionFileRead | — | in-pr4 |
| `agents.file-read.no-skill` | No skill content returned. | agentSessionFileRead | — | in-pr4 |
| `agents.harness.adapter-default` | Adapter default | EffortPickerField, effortPicker | — | in-pr4 |
| `agents.harness.add-custom` | Add custom harness… | addCustomHarness | — | in-pr4 |
| `agents.harness.add-custom-description` | Register any ACP-speaking agent tool as a selectable harness. | AddCustomHarnessDialog | — | in-pr4 |
| `agents.harness.add-custom-title` | Add custom harness | AddCustomHarnessDialog | — | in-pr4 |
| `agents.harness.detect-failed` | Couldn't detect agent harnesses. | HarnessCatalogRetryNotice | — | in-pr4 |
| `agents.harness.field-label` | Agent harness | AgentHarnessField | — | in-pr4 |
| `agents.identity-fields.agent-name` | Agent name | AgentDescriptionField | — | in-pr4 |
| `agents.identity-fields.description-placeholder` | What this agent does, in a sentence | AgentDescriptionField | — | in-pr4 |
| `agents.log-panel.copy` | Copy log | ManagedAgentLogPanel | — | in-pr4 |
| `agents.log-panel.no-output` | No log output yet. | ManagedAgentLogPanel | — | in-pr4 |
| `agents.log-panel.none-selected` | No local agent selected | ManagedAgentLogPanel | — | in-pr4 |
| `agents.log-panel.pick-agent` | Pick a managed agent to view the latest ACP log output. | ManagedAgentLogPanel | — | in-pr4 |
| `agents.log-panel.select-agent` | Select a local agent to inspect recent output. | ManagedAgentLogPanel | — | in-pr4 |
| `agents.log-panel.title` | Harness Log | ManagedAgentLogPanel | — | in-pr4 |
| `agents.managed-actions.bulk-confirm_one` | {{action}} {{count}} agent? | useManagedAgentActions | — | in-pr4 |
| `agents.managed-actions.bulk-confirm_other` | {{action}} {{count}} agents? | useManagedAgentActions | — | in-pr4 |
| `agents.managed-actions.bulk-failures` | {{failed}} of {{total}} failed. | useManagedAgentActions | — | in-pr4 |
| `agents.mcp-servers.disabled` | Disabled | McpServersSection | — | in-pr4 |
| `agents.mcp-servers.empty` | No custom servers configured. | McpServersSection | — | in-pr4 |
| `agents.mcp-servers.enabled` | Enabled | McpServersSection | — | in-pr4 |
| `agents.mcp-servers.title` | MCP servers | McpServersSection | — | in-pr4 |
| `agents.model-combobox.no-match` | No models match | PersonaModelCombobox | — | in-pr4 |
| `agents.model-combobox.search-aria` | Search models | PersonaModelCombobox | — | in-pr4 |
| `agents.model-combobox.search-placeholder` | Search models… | PersonaModelCombobox | — | in-pr4 |
| `agents.model-discovery.agent-possessive` | agent's | personaModelDiscoveryStatus | — | in-pr4 |
| `agents.model-discovery.anthropic-key-required` | Enter an Anthropic API key to load Anthropic models. | personaModelDiscoveryStatus | — | in-pr4 |
| `agents.model-discovery.default-auto` | Default (auto) | EditAgentProviderModelFields, usePersonaModelDiscovery | — | in-pr4 |
| `agents.model-discovery.default-model` | Default model | EditAgentProviderModelFields, PersonaModelField, usePersonaModelDiscovery | — | in-pr4 |
| `agents.model-discovery.default-model-named` | Default model ({{model}}) | usePersonaModelDiscovery | — | in-pr4 |
| `agents.model-discovery.empty-report` | {{agent}} reported no models. Check that the CLI is installed and signed in, then reopen this screen. | usePersonaModelDiscovery | — | in-pr4 |
| `agents.model-discovery.openai-key-required` | Enter an OpenAI runtime API key (OPENAI_COMPAT_API_KEY) to load OpenAI models. | personaModelDiscoveryStatus | — | in-pr4 |
| `agents.model-discovery.shared-compute-malformed` | Buzz received an invalid shared compute status. Check the member machine, then try again. | personaModelDiscoveryStatus | — | in-pr4 |
| `agents.model-discovery.shared-compute-relay-failed` | Buzz couldn't check shared compute through the relay. Check your relay connection and try again. | personaModelDiscoveryStatus | — | in-pr4 |
| `agents.model-discovery.shared-compute-unsupported` | This version of Buzz cannot use shared compute. Update Buzz or choose another provider. | personaModelDiscoveryStatus | — | in-pr4 |
| `agents.model-discovery.sign-in-required` | {{agent}} requires sign-in before models can load. Sign in with the {{cliOwner}} CLI in a terminal, then try again. | personaModelDiscoveryStatus | — | in-pr4 |
| `agents.model-discovery.this-agent` | This agent | personaModelDiscoveryStatus, usePersonaModelDiscovery | — | in-pr4 |
| `agents.model-field.choose` | Choose a model | PersonaModelField | — | in-pr4 |
| `agents.model-field.custom-id` | Custom model ID | EditAgentProviderModelFields, PersonaModelField | — | in-pr4 |
| `agents.model-field.shared-compute-hint` | Auto uses Mesh collective intelligence when two or more models stay available, otherwise it chooses one available model. | PersonaModelField | — | in-pr4 |
| `agents.model-picker.auto-collective` | Auto (collective when available) | relayMeshModelPicker | — | in-pr4 |
| `agents.model-picker.custom` | Custom model... | relayMeshModelPicker | — | in-pr4 |
| `agents.model-picker.loading` | Loading models... | relayMeshModelPicker | — | in-pr4 |
| `agents.model-tuning.context-limit` | Context limit | buzzAgentModelTuningFields | — | in-pr4 |
| `agents.model-tuning.context-limit-help` | Maximum context window tokens tracked before a handoff. Leave blank to inherit. | buzzAgentModelTuningFields | — | in-pr4 |
| `agents.model-tuning.inherit-agent-default` | Inherit (agent default) | buzzAgentModelTuningFields | — | in-pr4 |
| `agents.model-tuning.max-output-tokens` | Max output tokens | buzzAgentModelTuningFields | — | in-pr4 |
| `agents.model-tuning.max-output-tokens-help` | Maximum tokens the LLM may generate per response. Leave blank to inherit. | buzzAgentModelTuningFields | — | in-pr4 |
| `agents.model-tuning.max-rounds` | Max rounds | buzzAgentModelTuningFields | — | in-pr4 |
| `agents.model-tuning.max-rounds-help` | Maximum LLM + tool-call rounds per turn. 0 = unlimited. Leave blank to inherit. | buzzAgentModelTuningFields | — | in-pr4 |
| `agents.model-tuning.section` | buzz-agent model tuning | buzzAgentModelTuningFields | — | in-pr4 |
| `agents.model-tuning.thinking-effort` | Thinking / Effort | buzzAgentModelTuningFields | — | in-pr4 |
| `agents.model-tuning.thinking-effort-help` | Controls how much reasoning effort the LLM applies per turn. Leave blank to inherit from the global or persona default. | buzzAgentModelTuningFields | — | in-pr4 |
| `agents.persona-actions.managed-by-team` | Managed by team | PersonaActionsMenu | — | in-pr4 |
| `agents.persona-actions.open-aria` | Open actions for {{name}} | PersonaActionsMenu | — | in-pr4 |
| `agents.persona-added-by.label` | Added by | PersonaAddedBy | — | in-pr4 |
| `agents.persona-delete.cascade_one` | Also deletes {{count}} agent instance and archives its identity on the relay, so it no longer appears in member lists or mention suggestions. | PersonaDeleteDialog | — | in-pr4 |
| `agents.persona-delete.cascade_other` | Also deletes {{count}} agent instances and archives their identities on the relay, so they no longer appear in member lists or mention suggestions. | PersonaDeleteDialog | — | in-pr4 |
| `agents.persona-delete.confirm-name` | Delete {{name}}. | PersonaDeleteDialog | — | in-pr4 |
| `agents.persona-delete.confirm-no-persona` | Delete this agent. | PersonaDeleteDialog | — | in-pr4 |
| `agents.persona-delete.title` | Delete agent? | PersonaDeleteDialog | — | in-pr4 |
| `agents.profile-sync.warning` | {{agentName}} was saved locally, but relay sync failed: {{error}}. Remote users may still see the previous name or access policy until Buzz retries the sync. | agentProfileSyncWarning | — | in-pr4 |
| `agents.prompt-section.no-metadata` | No metadata. | PromptSectionAccordion, SentMessageContextDialog | — | in-pr4 |
| `agents.raw-events.empty` | No raw events yet. | RawEventRail | — | in-pr4 |
| `agents.restart-diff.changed` | Config changed since last start: | RestartDiffBadge | — | in-pr4 |
| `agents.restart-diff.required` | Restart required | RestartDiffBadge | — | in-pr4 |
| `agents.run-on.label` | Run on | RunOnSummarySection, WhereToRunSection | — | in-pr4 |
| `agents.run-on.no-saved-settings` | No saved settings — the provider applies its defaults. | RunOnSummarySection | — | in-pr4 |
| `agents.run-on.read-only-note` | These are the settings saved when the agent was created. Where an agent runs can't be changed afterwards — create a new agent to run somewhere else. | RunOnSummarySection | — | in-pr4 |
| `agents.run-on.this-computer` | This computer | RunOnSummarySection, WhereToRunSection | — | in-pr4 |
| `agents.sent-context.title` | Sent message context | SentMessageContextDialog | — | in-pr4 |
| `agents.share-recipients.add-aria` | Add {{name}} | PersonaShareRecipients | — | in-pr4 |
| `agents.share-recipients.limit-reached` | Recipient limit reached | PersonaShareRecipients | — | in-pr4 |
| `agents.share-recipients.loading-aria` | Loading people | PersonaShareRecipients | — | in-pr4 |
| `agents.share-recipients.no-people` | No people found. | PersonaShareRecipients | — | in-pr4 |
| `agents.share-recipients.share-with-aria` | Share with | PersonaShareRecipients | — | in-pr4 |
| `agents.team-card.linked-from` | Linked from {{path}} | TeamIdentityCard | — | in-pr4 |
| `agents.team-card.member-avatars-aria` | {{teamName}} member avatars | TeamIdentityCard | — | in-pr4 |
| `agents.team-delete.confirm-name` | Delete "{{name}}". Already-deployed agents are not affected, but this team template will no longer be available. | TeamDeleteDialog | — | in-pr4 |
| `agents.team-delete.confirm-no-team` | Delete this team. | TeamDeleteDialog | — | in-pr4 |
| `agents.team-delete.title` | Delete team? | TeamDeleteDialog | — | in-pr4 |
| `agents.team-dialog.name-placeholder` | Engineering Squad | TeamDialog | — | in-pr4 |
| `agents.teams-section.missing-in-team_one` | {{count}} agent in this team is no longer in your agents. Edit the team to fix it before deploying or sharing. | TeamsSection | — | in-pr4 |
| `agents.teams-section.missing-in-team_other` | {{count}} agents in this team are no longer in your agents. Edit the team to fix it before deploying or sharing. | TeamsSection | — | in-pr4 |
| `agents.tool-details.parameters` | Parameters | SentMessageContextDialog, ToolDetailBlocks | — | in-pr4 |
| `agents.tool-details.result` | Result | SentMessageContextDialog, ToolDetailBlocks | — | in-pr4 |
| `agents.tool-details.waiting` | Waiting for tool details. | SentMessageContextDialog, ToolDetailBlocks | — | in-pr4 |
| `agents.tool-summary.no-todos` | No todos. | TodoToolSummary | — | in-pr4 |
| `agents.tool-summary.show-sent-context` | Show sent message context | CompactMessageSummary | — | in-pr4 |
| `agents.transcript-list.hide-prompt-context` | Hide prompt context | AgentSessionTranscriptList | — | in-pr4 |
| `agents.transcript-list.live-aria` | Live ACP transcript | AgentSessionTranscriptList | — | in-pr4 |
| `agents.transcript-list.no-activity` | No ACP activity yet | AgentSessionTranscriptList | — | in-pr4 |
| `agents.transcript-list.prompt-context` | Prompt context | AgentSessionTranscriptList | — | in-pr4 |
| `agents.transcript-list.session-earlier` | Earlier observed session | AgentSessionTranscriptList | — | in-pr4 |
| `agents.transcript-list.session-latest` | Latest live-observed session | AgentSessionTranscriptList | — | in-pr4 |
| `agents.transcript-list.session-most-recent` | Most recent observed session | AgentSessionTranscriptList | — | in-pr4 |
| `agents.transcript-list.show-prompt-context` | Show prompt context | AgentSessionTranscriptList | — | in-pr4 |
| `agents.transcript-list.waiting-aria` | Waiting for ACP activity | AgentSessionTranscriptList | — | in-pr4 |
| `agents.transcript-list.wire-source` | ACP wire source: {{source}} | AgentSessionTranscriptList | — | in-pr4 |
| `agents.transcript.agent-error-crash` | Agent error (crash) | agentSessionTranscript | — | in-pr4 |
| `agents.transcript.edited-files_one` | Edited {{count}} file | agentSessionTranscriptGrouping | — | in-pr4 |
| `agents.transcript.edited-files_other` | Edited {{count}} files | agentSessionTranscriptGrouping | — | in-pr4 |
| `agents.transcript.item-count` | {{count}} items | agentSessionTranscriptGrouping | — | in-pr4 |
| `agents.transcript.open-in-chat` | Open in chat | MessageLinkHoverCue | — | in-pr4 |
| `agents.transcript.open-profile-aria` | Open {{name}} profile | CompactMessageSummary, UserMessageBubble | — | in-pr4 |
| `agents.transcript.permission-requested` | Permission requested | agentSessionTranscript | — | in-pr4 |
| `agents.transcript.plan-updated` | Plan updated | agentSessionTranscript | — | in-pr4 |
| `agents.transcript.ran-commands` | Ran {{count}} commands | agentSessionTranscriptGrouping | — | in-pr4 |
| `agents.transcript.ran-relay-ops` | Ran {{count}} Buzz relay ops | agentSessionTranscriptGrouping | — | in-pr4 |
| `agents.transcript.ran-tool-calls` | Ran {{count}} tool calls | agentSessionTranscriptGrouping | — | in-pr4 |
| `agents.transcript.read-files` | Read {{count}} files | agentSessionTranscriptGrouping | — | in-pr4 |
| `agents.transcript.read-skills_one` | Read {{count}} skill | agentSessionTranscriptGrouping | — | in-pr4 |
| `agents.transcript.read-skills_other` | Read {{count}} skills | agentSessionTranscriptGrouping | — | in-pr4 |
| `agents.transcript.turn-error` | Turn error | agentSessionTranscript | — | in-pr4 |
| `agents.transcript.turn-in-progress` | Agent turn in progress | TurnLivenessIndicator | — | in-pr4 |
| `agents.unified-section.configuration-missing` | Configuration missing | UnifiedAgentsSection | — | in-pr4 |
| `agents.unified-section.group-custom` | Custom agents | UnifiedAgentsSection | — | in-pr4 |
| `agents.unified-section.group-unknown` | Unknown agents | UnifiedAgentsSection | — | in-pr4 |
| `agents.unified-section.profile-aria` | {{title}} agent profile | UnifiedAgentsSection | — | in-pr4 |
| `agents.view.actions-aria` | Agent actions | AgentsView | — | in-pr4 |
| `agents.view.description` | Set up and manage your agents. | AgentsView | — | in-pr4 |
| `agents.view.set-agent-defaults` | Set agent defaults | AgentsView | — | in-pr4 |
| `agents.view.stop-running-agents` | Stop running agents | AgentsView | — | in-pr4 |
| `agents.where-to-run.key-warning` | This provider at {{path}} will receive your agent's private key. Only use providers from trusted sources. | WhereToRunSection | — | in-pr4 |
| `agents.where-to-run.placeholder` | Choose where to run | WhereToRunSection | — | in-pr4 |
| `agents.where-to-run.probe-failed` | Could not probe provider: {{error}} | WhereToRunSection | — | in-pr4 |

| `profile.animated.poster-help` | Pick the still shown before hover. | AnimatedAvatarCapture | — | in-pr4 |
| `profile.animated.preview-aria` | Avatar preview — drag or use arrow keys to position | AnimatedAvatarCapture | — | in-pr4 |
| `profile.animated.processing` | Processing recording | AnimatedAvatarCapture | — | in-pr4 |
| `profile.animated.starting` | Starting camera | AnimatedAvatarCapture | — | in-pr4 |
| `profile.animated.uploading` | Uploading animated avatar | AnimatedAvatarCapture | — | in-pr4 |
| `profile.animated.use-as-avatar` | Use as avatar | AnimatedAvatarCapture | — | in-pr4 |
| `profile.avatar-controls.outline-off-aria` | Turn outline off | AnimatedAvatarControls | — | in-pr4 |
| `profile.avatar-controls.outline-off-title` | Outline off | AnimatedAvatarControls | — | in-pr4 |
| `profile.avatar-controls.outline-on-aria` | Turn outline on | AnimatedAvatarControls | — | in-pr4 |
| `profile.avatar-controls.outline-on-title` | Outline on | AnimatedAvatarControls | — | in-pr4 |
| `profile.avatar-controls.still-frame-aria` | Choose still frame | AnimatedAvatarControls | — | in-pr4 |
| `profile.avatar-controls.thumbnails-aria` | Generating frame thumbnails | AnimatedAvatarControls | — | in-pr4 |
| `profile.avatar-editor.color-aria` | Choose custom avatar color | ProfileAvatarEditor | — | in-pr4 |
| `profile.avatar-editor.color-needs-emoji-aria` | Choose an emoji before custom avatar color | ProfileAvatarEditor | — | in-pr4 |
| `profile.avatar-editor.done` | Done | ProfileAvatarEditor | — | in-pr4 |
| `profile.avatar-editor.drop-or` | Drop or  | ProfileAvatarEditor | — | in-pr4 |
| `profile.avatar-editor.legend` | Avatar image picker | ProfileAvatarEditor | — | in-pr4 |
| `profile.avatar-editor.save` | Save | ProfileAvatarEditor | — | in-pr4 |
| `profile.avatar-editor.saving` | Saving | ProfileAvatarEditor | — | in-pr4 |
| `profile.avatar-editor.saving-aria` | Saving avatar | ProfileAvatarEditor | — | in-pr4 |
| `profile.avatar-editor.url-placeholder` | Paste a URL | ProfileAvatarEditor | — | in-pr4 |
| `profile.avatar-editor.url-placeholder-detail` | Paste a URL (Slack profile, etc.) | ProfileAvatarEditor | — | in-pr4 |
| `profile.avatar-editor.use-color-aria` | Use {{color}} background | ProfileAvatarEditor | — | in-pr4 |
| `profile.avatar-upload.drop` | Drop an image or  | AvatarUpload | — | in-pr4 |
| `profile.avatar-upload.heading` | Add a profile photo | AvatarUpload | — | in-pr4 |
| `profile.avatar-upload.remove-title` | Remove photo | AvatarUpload | — | in-pr4 |
| `profile.avatar-upload.url-hint` | Or paste a direct image URL. | AvatarUpload | — | in-pr4 |
| `profile.avatar-upload.url-label` | Avatar URL | AvatarUpload | — | in-pr4 |
| `profile.avatar.pending-aria` | Avatar upload pending | ProfileAvatar | — | in-pr4 |
| `profile.backdrop.custom-aria` | Choose custom backdrop color | AnimatedAvatarBackdropPanel | — | in-pr4 |
| `profile.backdrop.use-color-aria` | Use {{color}} backdrop | AnimatedAvatarBackdropPanel | — | in-pr4 |
| `profile.camera.capture` | Capture {{seconds}} sec video | AnimatedAvatarCameraControls | — | in-pr4 |
| `profile.camera.retry` | Try camera again | AnimatedAvatarCameraControls | — | in-pr4 |
| `profile.channels-view.add-row` | Add to channel | UserProfilePanelFocusedViews | — | in-pr4 |
| `profile.channels-view.empty-cta` | Add this agent to a channel | UserProfilePanelFocusedViews | — | in-pr4 |
| `profile.channels-view.empty-hint-select` | Choose a channel above so it can join the conversation. | UserProfilePanelFocusedViews | — | in-pr4 |
| `profile.channels-view.empty-hint-visible` | Visible memberships appear as this agent joins channels. | UserProfilePanelFocusedViews | — | in-pr4 |
| `profile.channels-view.empty-title` | Channels appear here | UserProfilePanelFocusedViews | — | in-pr4 |
| `profile.channels-view.loading` | Loading channels… | UserProfilePanelFocusedViews | — | in-pr4 |
| `profile.channels-view.open-aria` | Open #{{name}} | UserProfilePanelFocusedViews | — | in-pr4 |
| `profile.color-panel.hue-aria` | Choose custom avatar color hue | AvatarCustomColorPanel | — | in-pr4 |
| `profile.color-panel.use-color` | Use color | AvatarCustomColorPanel | — | in-pr4 |
| `profile.menu.presence-aria` | Presence status | ProfilePopover | — | in-pr4 |
| `profile.menu.profile-aria` | Profile menu | ProfilePopover | — | in-pr4 |
| `profile.menu.send-feedback` | Send feedback | ProfilePopover | — | in-pr4 |
| `profile.menu.settings` | Settings | ProfilePopover | — | in-pr4 |
| `profile.menu.update-status` | Update your status | ProfilePopover | — | in-pr4 |
| `profile.nostr-bind.browser-description` | Buzz opened your browser to finish verification. | NostrBindConsentDialog | — | in-pr4 |
| `profile.nostr-bind.browser-title` | Continue in your browser | NostrBindConsentDialog | — | in-pr4 |
| `profile.nostr-bind.cancel` | Cancel | NostrBindConsentDialog | — | in-pr4 |
| `profile.nostr-bind.code-description` | Enter the six-digit code shown in your browser | NostrBindConsentDialog | — | in-pr4 |
| `profile.nostr-bind.code-legend` | Verification code | NostrBindConsentDialog | — | in-pr4 |
| `profile.nostr-bind.code-title` | Enter verification code | NostrBindConsentDialog | — | in-pr4 |
| `profile.nostr-bind.continue` | Continue | NostrBindConsentDialog | — | in-pr4 |
| `profile.nostr-bind.copied` | Copied | NostrBindConsentDialog | — | in-pr4 |
| `profile.nostr-bind.copy-response` | Copy response | NostrBindConsentDialog | — | in-pr4 |
| `profile.nostr-bind.digit-aria` | Verification code digit {{index}} of {{total}} | NostrBindConsentDialog | — | in-pr4 |
| `profile.nostr-bind.fallback-description` | Copy this response and paste it into the pairing page. | NostrBindConsentDialog | — | in-pr4 |
| `profile.nostr-bind.fallback-summary` | Pairing didn’t finish automatically? | NostrBindConsentDialog | — | in-pr4 |
| `profile.nostr-bind.signing` | Signing… | NostrBindConsentDialog | — | in-pr4 |
| `profile.nostr-bind.site-description` | Copy the response below, then paste it into the Buzz website to finish verification. | NostrBindConsentDialog | — | in-pr4 |
| `profile.nostr-bind.site-title` | Finish on the Buzz website | NostrBindConsentDialog | — | in-pr4 |
| `profile.panel.added-to-channel` | Added {{name}} to {{channel}}. | UserProfilePanel | — | in-pr4 |
| `profile.panel.already-in-channel` | {{name}} is already in {{channel}}. | UserProfilePanel | — | in-pr4 |
| `profile.panel.delete-failed` | Failed to delete agent. | UserProfilePanel | — | in-pr4 |
| `profile.panel.deleted` | Deleted {{name}}. | UserProfilePanel | — | in-pr4 |
| `profile.panel.info-title` | Info | UserProfilePanelFocusedViews | — | in-pr4 |
| `profile.panel.manual-start-only` | {{name}} will stay manual-start only. | UserProfilePanel | — | in-pr4 |
| `profile.panel.removed-from-my-agents` | Removed {{name}} from My Agents. | UserProfilePanel | — | in-pr4 |
| `profile.panel.start-failed` | Failed to start agent. | UserProfilePanel | — | in-pr4 |
| `profile.panel.startup-preference-failed` | Failed to update startup preference. | UserProfilePanel | — | in-pr4 |
| `profile.panel.team-managed` | This agent is managed by a team. | UserProfilePanel | — | in-pr4 |
| `profile.panel.will-start-automatically` | Will start {{name}} automatically. | UserProfilePanel | — | in-pr4 |
| `profile.review-nav.background` | Background | AnimatedAvatarReviewNav | — | in-pr4 |
| `profile.review-nav.caption-circle` | Circle | AnimatedAvatarReviewNav | — | in-pr4 |
| `profile.review-nav.caption-frame` | Frame | AnimatedAvatarReviewNav | — | in-pr4 |
| `profile.review-nav.caption-you` | You | AnimatedAvatarReviewNav | — | in-pr4 |
| `profile.review-nav.label-adjust-circle` | Adjust the circle | AnimatedAvatarReviewNav | — | in-pr4 |
| `profile.review-nav.label-person` | Position yourself | AnimatedAvatarReviewNav | — | in-pr4 |
| `profile.review-nav.label-still-frame` | Still frame | AnimatedAvatarReviewNav | — | in-pr4 |
| `profile.review-nav.retake` | Retake | AnimatedAvatarReviewNav | — | in-pr4 |
| `profile.review-nav.retake-aria` | Retake the recording | AnimatedAvatarReviewNav | — | in-pr4 |
| `profile.upload.failed` | Avatar couldn’t finish uploading | avatarPresentationStore | — | in-pr4 |
| `profile.upload.failed-description` | Your default avatar is showing instead. | avatarPresentationStore | — | in-pr4 |
| `profile.upload.retry` | Retry | avatarPresentationStore | — | in-pr4 |

| `settings.appearance.accent.hint` | Choose the highlight color used throughout Buzz. | AppearanceSettingsControls | — | in-pr4 |
| `settings.appearance.accent.label` | Accent color | AppearanceSettingsControls | — | in-pr4 |
| `settings.appearance.accent.swatch-aria` | Use {{name}} accent | AppearanceSettingsControls | — | in-pr4 |
| `settings.appearance.color-mode.hint` | Follow your system or choose a light or dark appearance. | SettingsPanels | — | in-pr4 |
| `settings.appearance.color-mode.label` | Color mode | SettingsPanels | — | in-pr4 |
| `settings.appearance.color-mode.option-dark` | Dark | SettingsPanels | — | in-pr4 |
| `settings.appearance.color-mode.option-light` | Light | SettingsPanels | — | in-pr4 |
| `settings.appearance.color-mode.option-system` | System | SettingsPanels | — | in-pr4 |
| `settings.appearance.density.hint` | Spacing in conversations and Markdown content across Buzz | AppearanceSettingsControls | — | in-pr4 |
| `settings.appearance.density.label` | Conversation density | AppearanceSettingsControls | — | in-pr4 |
| `settings.appearance.density.option-comfy` | Comfy | AppearanceSettingsControls | — | in-pr4 |
| `settings.appearance.density.option-compact` | Compact | AppearanceSettingsControls | — | in-pr4 |
| `settings.appearance.density.option-spacious` | Spacious | AppearanceSettingsControls | — | in-pr4 |
| `settings.appearance.glass.hint` | Blur the desktop behind navigation while keeping content solid. | AppearanceSettingsControls | — | in-pr4 |
| `settings.appearance.glass.label` | Glass background | AppearanceSettingsControls | — | in-pr4 |
| `settings.appearance.glass.opacity-hint` | Lower values reveal more of the desktop blur. | AppearanceSettingsControls | — | in-pr4 |
| `settings.appearance.glass.opacity-label` | Glass opacity | AppearanceSettingsControls | — | in-pr4 |
| `settings.appearance.glass.unsupported-hint` | Available in the macOS desktop app. | AppearanceSettingsControls | — | in-pr4 |
| `settings.appearance.link-previews.compact-hint` | Small cards with a thumbnail | AppearanceSettingsControls | — | in-pr4 |
| `settings.appearance.link-previews.label` | Link previews | AppearanceSettingsControls | — | in-pr4 |
| `settings.appearance.link-previews.rich-hint` | Large previews with images and descriptions | AppearanceSettingsControls | — | in-pr4 |
| `settings.appearance.page-description` | Choose how Buzz looks and feels. | SettingsPanels | — | in-pr4 |
| `settings.appearance.preview.label` | Preview | AppearanceSettingsControls | — | in-pr4 |
| `settings.appearance.preview.sample-link-description` | Highlights from this release: refreshed conversation layout, quicker link handling, and readability improvements. | AppearanceSettingsControls | — | in-pr4 |
| `settings.appearance.preview.sample-link-title` | Product updates — a fresh look at conversations | AppearanceSettingsControls | — | in-pr4 |
| `settings.appearance.preview.sample-message-1` | The revised conversation layout is ready to review. | AppearanceSettingsControls | — | in-pr4 |
| `settings.appearance.preview.sample-message-2` | I added a longer message so you can compare line height and text spacing. | AppearanceSettingsControls | — | in-pr4 |
| `settings.appearance.preview.sample-message-3` | The same rhythm carries through channels, threads, DMs, and Inbox. | AppearanceSettingsControls | — | in-pr4 |
| `settings.appearance.prominent-tab.hint` | Give the selected navigation item a higher-contrast background. | AppearanceSettingsControls | — | in-pr4 |
| `settings.appearance.prominent-tab.label` | Prominent active tab | AppearanceSettingsControls | — | in-pr4 |
| `settings.appearance.theme-style.aria` | Theme style, {{theme}} | SettingsPanels | — | in-pr4 |
| `settings.appearance.theme-style.hint` | Choose the colors used throughout Buzz. | SettingsPanels | — | in-pr4 |
| `settings.appearance.theme-style.label` | Theme style | SettingsPanels | — | in-pr4 |
| `settings.appearance.theme.label` | Theme | SettingsPanels | — | in-pr4 |
| `settings.appearance.theme.per-community` | (per community) | SettingsPanels | — | in-pr4 |
| `settings.appearance.thread-layout.focus-hint` | Threads open over the channel | AppearanceSettingsControls | — | in-pr4 |
| `settings.appearance.thread-layout.label` | Thread layout | AppearanceSettingsControls | — | in-pr4 |
| `settings.appearance.thread-layout.option-focus` | Focus | AppearanceSettingsControls | — | in-pr4 |
| `settings.appearance.thread-layout.option-split` | Split | AppearanceSettingsControls | — | in-pr4 |
| `settings.appearance.thread-layout.scope` | (all communities) | AppearanceSettingsControls | — | in-pr4 |
| `settings.appearance.thread-layout.split-hint` | Threads open in a side panel next to the channel | AppearanceSettingsControls | — | in-pr4 |
| `settings.common.cancel` | Cancel | CustomHarnessForm, HarnessRow | — | in-pr4 |
| `settings.common.delete` | Delete | HarnessRow | — | in-pr4 |
| `settings.common.edit` | Edit | HarnessRow | — | in-pr4 |
| `settings.common.group-preferences` | Preferences | PreventSleepSettingsCard, SettingsPanels | — | in-pr4 |
| `settings.common.retry` | Retry | HarnessCatalogDialog | — | in-pr4 |
| `settings.common.save` | Save | CustomHarnessForm | — | in-pr4 |
| `settings.common.try-again` | Try again | SettingsView | — | in-pr4 |
| `settings.custom-harness.add-arg` | Add argument | CustomHarnessForm | — | in-pr4 |
| `settings.custom-harness.add-env` | Add env var | CustomHarnessForm | — | in-pr4 |
| `settings.custom-harness.arguments` | Arguments | CustomHarnessForm | — | in-pr4 |
| `settings.custom-harness.command` | Command | CustomHarnessForm | — | in-pr4 |
| `settings.custom-harness.command-hint` | Any command that speaks ACP over stdio works. | CustomHarnessForm | — | in-pr4 |
| `settings.custom-harness.docs-url` | Docs URL | CustomHarnessForm | — | in-pr4 |
| `settings.custom-harness.edit-title` | Edit harness | CustomHarnessForm | — | in-pr4 |
| `settings.custom-harness.env-vars` | Env vars | CustomHarnessForm | — | in-pr4 |
| `settings.custom-harness.found-on-path` | Found on PATH | CustomHarnessForm | — | in-pr4 |
| `settings.custom-harness.install-hint` | Install hint | CustomHarnessForm | — | in-pr4 |
| `settings.custom-harness.name` | Name | CustomHarnessForm | — | in-pr4 |
| `settings.custom-harness.not-found-on-path` | Not found on PATH | CustomHarnessForm | — | in-pr4 |
| `settings.custom-harness.remove-arg-aria` | Remove argument | CustomHarnessForm | — | in-pr4 |
| `settings.custom-harness.remove-env-aria` | Remove env var | CustomHarnessForm | — | in-pr4 |
| `settings.prevent-sleep.expired` | Sleep prevention expired after 1 hour without agent activity. It will resume on the next agent activity, or toggle off and on to re-enable now. | PreventSleepSettingsCard | — | in-pr4 |
| `settings.prevent-sleep.hint` | Prevents your computer from sleeping while local agents are running. Automatically releases when all agents stop or after 1 hour without agent activity. | PreventSleepSettingsCard | — | in-pr4 |
| `settings.prevent-sleep.label` | Keep awake while agents are active | PreventSleepSettingsCard | — | in-pr4 |
| `settings.prevent-sleep.waiting` | Waiting for agents to start | PreventSleepSettingsCard | — | in-pr4 |
| `settings.runtimes.action-needed` | Action needed | HarnessesSettingsPanel | — | in-pr4 |
| `settings.runtimes.add-button` | Add runtimes | HarnessesSettingsPanel | — | in-pr4 |
| `settings.runtimes.available` | Available | HarnessesSettingsPanel | — | in-pr4 |
| `settings.runtimes.catalog.empty` | No runtimes found. | HarnessCatalogDialog | — | in-pr4 |
| `settings.runtimes.catalog.load-failed` | Couldn't load runtimes. | HarnessCatalogDialog | — | in-pr4 |
| `settings.runtimes.catalog.no-match` | No runtimes match. | HarnessCatalogDialog | — | in-pr4 |
| `settings.runtimes.catalog.refresh-failed` | Couldn't refresh runtimes. | HarnessCatalogDialog | — | in-pr4 |
| `settings.runtimes.catalog.refreshing` | Refreshing… | HarnessCatalogDialog | — | in-pr4 |
| `settings.runtimes.catalog.search-aria` | Search runtimes | HarnessCatalogDialog | — | in-pr4 |
| `settings.runtimes.catalog.search-placeholder` | Search runtimes… | HarnessCatalogDialog | — | in-pr4 |
| `settings.runtimes.catalog.section-installed` | Installed | HarnessCatalogDialog | — | in-pr4 |
| `settings.runtimes.catalog.section-setup` | Setup | HarnessCatalogDialog | — | in-pr4 |
| `settings.runtimes.catalog.title` | Add runtimes | HarnessCatalogDialog | — | in-pr4 |
| `settings.runtimes.check-again` | Check again | HarnessesSettingsPanel | — | in-pr4 |
| `settings.runtimes.checking` | Checking agent runtimes... | HarnessesSettingsPanel | — | in-pr4 |
| `settings.runtimes.delete-check-failed` | Couldn't check which agents use this harness — some agents may stop launching. Delete {{label}}? | harnessGalleryLogic | — | in-pr4 |
| `settings.runtimes.delete-checking` | Checking which agents use this harness… | harnessGalleryLogic | — | in-pr4 |
| `settings.runtimes.description` | Choose which agent tools Buzz can use on this device. | HarnessesSettingsPanel | — | in-pr4 |
| `settings.runtimes.git-bash-required` | Required for buzz-agent shell tools on Windows. | HarnessesSettingsPanel | — | in-pr4 |
| `settings.runtimes.install` | Install | HarnessRow, harnessCatalogLogic | — | in-pr4 |
| `settings.runtimes.install-git-bash` | Install Git for Windows | HarnessesSettingsPanel | — | in-pr4 |
| `settings.runtimes.none-ready` | No agent runtimes ready yet — add one below. | HarnessesSettingsPanel | — | in-pr4 |
| `settings.runtimes.prerequisites-hint` | Windows tools required by supported agents. | HarnessesSettingsPanel | — | in-pr4 |
| `settings.runtimes.prerequisites-title` | System prerequisites | HarnessesSettingsPanel | — | in-pr4 |
| `settings.runtimes.ready` | Ready | HarnessRow | — | in-pr4 |
| `settings.runtimes.row.actions-aria` | Open actions for {{label}} | HarnessRow | — | in-pr4 |
| `settings.runtimes.row.config-error` | Config error: {{diagnostic}} | HarnessRow | — | in-pr4 |
| `settings.runtimes.row.finish-signin` | Finish signing in from the Terminal window, then click Check again to re-check {{label}}. | HarnessRow | — | in-pr4 |
| `settings.runtimes.row.install-aria` | Install {{label}} | HarnessRow | — | in-pr4 |
| `settings.runtimes.row.install-node` | Install Node.js | HarnessRow | — | in-pr4 |
| `settings.runtimes.title` | Agent runtimes | HarnessesSettingsPanel | — | in-pr4 |
| `settings.runtimes.update` | Update | HarnessRow, harnessCatalogLogic | — | in-pr4 |
| `settings.runtimes.update-adapter-title` | Update {{label}} adapter? | HarnessRow | — | in-pr4 |
| `settings.runtimes.your-subtitle` | Ready to use, or one click from installed. | HarnessesSettingsPanel | — | in-pr4 |
| `settings.runtimes.your-title` | Your runtimes | HarnessesSettingsPanel | — | in-pr4 |
| `settings.sections.agents` | Agents | SettingsPanels | — | in-pr4 |
| `settings.sections.appearance` | Appearance | SettingsPanels | — | in-pr4 |
| `settings.sections.channel-templates` | Channel templates | SettingsPanels | — | in-pr4 |
| `settings.sections.compute` | Compute | SettingsPanels | — | in-pr4 |
| `settings.sections.custom-emoji` | Custom emoji | SettingsPanels | — | in-pr4 |
| `settings.sections.experiments` | Experiments | SettingsPanels | — | in-pr4 |
| `settings.sections.group-app` | App | SettingsView | — | in-pr4 |
| `settings.sections.group-aria` | {{group}} settings sections | SettingsView | — | in-pr4 |
| `settings.sections.group-communities` | Communities | SettingsView | — | in-pr4 |
| `settings.sections.group-personal` | Personal | SettingsView | — | in-pr4 |
| `settings.sections.hosted-communities` | Hosted communities | SettingsPanels | — | in-pr4 |
| `settings.sections.invites` | Invites | SettingsPanels | — | in-pr4 |
| `settings.sections.local-archive` | Local archive | SettingsPanels | — | in-pr4 |
| `settings.sections.mobile` | Mobile | SettingsPanels | — | in-pr4 |
| `settings.sections.moderation` | Moderation | SettingsPanels | — | in-pr4 |
| `settings.sections.notifications` | Notifications | SettingsPanels | — | in-pr4 |
| `settings.sections.profile` | Profile | SettingsPanels | — | in-pr4 |
| `settings.sections.shortcuts` | Shortcuts | SettingsPanels | — | in-pr4 |
| `settings.sections.updates` | Updates | SettingsPanels | — | in-pr4 |
| `settings.sections.voice` | Voice | SettingsPanels | — | in-pr4 |
| `settings.shortcuts.description` | All available keyboard shortcuts. Shortcuts are read-only. | KeyboardShortcutsCard | — | in-pr4 |
| `settings.shortcuts.title` | Keyboard shortcuts | KeyboardShortcutsCard | — | in-pr4 |
| `settings.sounds.pause-aria` | Pause {{sound}} | SoundPicker | — | in-pr4 |
| `settings.sounds.preview-aria` | Preview {{sound}} | SoundPicker | — | in-pr4 |
| `settings.view.back-to-app` | Back to app | SettingsView | — | in-pr4 |
| `settings.view.invite-check-failed` | Invite settings could not be checked. | SettingsView | — | in-pr4 |
| `settings.view.invite-settings-unavailable` | Invite settings are unavailable. Relay recovery may still be in progress. | SettingsView | — | in-pr4 |

| `agents.add-agent-to-channel.add` | Add to channel | AddAgentToChannelDialog | — | in-pr4 |
| `agents.add-agent-to-channel.adding` | Adding... | AddAgentToChannelDialog | — | in-pr4 |
| `agents.add-agent-to-channel.agent-pubkey` | Agent pubkey | AddAgentToChannelDialog | — | in-pr4 |
| `agents.add-agent-to-channel.already-member` | Already a member of this channel | AddAgentToChannelDialog | — | in-pr4 |
| `agents.add-agent-to-channel.cancel` | Cancel | AddAgentToChannelDialog | — | in-pr4 |
| `agents.add-agent-to-channel.channel` | Channel | AddAgentToChannelDialog | — | in-pr4 |
| `agents.add-agent-to-channel.channels-hint` | Only channels accessible to the current desktop user are shown here. | AddAgentToChannelDialog | — | in-pr4 |
| `agents.add-agent-to-channel.copy-pubkey` | Copy pubkey | AddAgentToChannelDialog | — | in-pr4 |
| `agents.add-agent-to-channel.description` | Add {{name}} to a channel so desktop chat can `@mention` it. Running agents pick up new channels automatically via membership notifications. | AddAgentToChannelDialog | — | in-pr4 |
| `agents.add-agent-to-channel.no-agent` | No agent selected | AddAgentToChannelDialog | — | in-pr4 |
| `agents.add-agent-to-channel.no-channels` | No channels available | AddAgentToChannelDialog | — | in-pr4 |
| `agents.add-agent-to-channel.re-add` | Re-add to channel | AddAgentToChannelDialog | — | in-pr4 |
| `agents.add-agent-to-channel.role` | Role | AddAgentToChannelDialog | — | in-pr4 |
| `agents.add-agent-to-channel.this-agent` | this agent | AddAgentToChannelDialog | — | in-pr4 |
| `agents.add-agent-to-channel.title` | Add agent to channel | AddAgentToChannelDialog | — | in-pr4 |
| `agents.add-team-to-channel.agents-count` | Agents ({{count}}) | AddTeamToChannelDialog | — | in-pr4 |
| `agents.add-team-to-channel.cancel` | Cancel | AddTeamToChannelDialog | — | in-pr4 |
| `agents.add-team-to-channel.channel` | Channel | AddTeamToChannelDialog | — | in-pr4 |
| `agents.add-team-to-channel.deploy_one` | Deploy {{count}} agent | AddTeamToChannelDialog | — | in-pr4 |
| `agents.add-team-to-channel.deploy_other` | Deploy {{count}} agents | AddTeamToChannelDialog | — | in-pr4 |
| `agents.add-team-to-channel.deploying` | Deploying... | AddTeamToChannelDialog | — | in-pr4 |
| `agents.add-team-to-channel.description-prefix` | Create and attach one agent per member of | AddTeamToChannelDialog | — | in-pr4 |
| `agents.add-team-to-channel.description-suffix` | to the selected channel. | AddTeamToChannelDialog | — | in-pr4 |
| `agents.add-team-to-channel.missing-personas_one` | This team references {{count}} agent that is no longer in My Agents. Add them back or edit the team before deploying. | AddTeamToChannelDialog | — | in-pr4 |
| `agents.add-team-to-channel.missing-personas_other` | This team references {{count}} agents that are no longer in My Agents. Add them back or edit the team before deploying. | AddTeamToChannelDialog | — | in-pr4 |
| `agents.add-team-to-channel.no-channels` | No channels available | AddTeamToChannelDialog | — | in-pr4 |
| `agents.add-team-to-channel.no-runtimes` | No ACP runtimes found. Make sure an agent runtime (e.g. Goose) is installed. | AddTeamToChannelDialog | — | in-pr4 |
| `agents.add-team-to-channel.role` | Role | AddTeamToChannelDialog | — | in-pr4 |
| `agents.add-team-to-channel.this-team` | this team | AddTeamToChannelDialog | — | in-pr4 |
| `agents.add-team-to-channel.title` | Deploy team to channel | AddTeamToChannelDialog | — | in-pr4 |
| `agents.card-mint.free-path-action` | Share without card art | AgentCardMintDialog | — | in-pr4 |
| `agents.card-mint.free-path-copy` | Don’t want to spend money? Ordinary export shares the same importable agent — free, just without the card art. | AgentCardMintDialog | — | in-pr4 |
| `agents.card-mint.key-save-failed` | Couldn’t save the key. | AgentCardMintDialog | — | in-pr4 |
| `agents.card-mint.key-saved` | API key saved to your agent defaults. Running agents pick it up on their next restart. | AgentCardMintDialog | — | in-pr4 |
| `agents.common.agent-all-memories` | Agent + all memories | AgentCardMintDialog, AgentSnapshotExportDialog | — | in-pr4 |
| `agents.common.agent-core-memory` | Agent + core memory | AgentCardMintDialog, AgentSnapshotExportDialog | — | in-pr4 |
| `agents.common.agent-only` | Agent only | AgentCardMintDialog, AgentSnapshotExportDialog | — | in-pr4 |
| `agents.common.export-title` | Export {{name}} | AgentSnapshotExportDialog, TeamSnapshotExportDialog | — | in-pr4 |
| `agents.common.file-format` | File format | AgentSnapshotExportDialog, TeamSnapshotExportDialog | — | in-pr4 |
| `agents.common.memories` | Memories | AgentSnapshotExportDialog, TeamSnapshotExportDialog | — | in-pr4 |
| `agents.common.memory-in-snapshot` | in the snapshot. Only share it with people you trust. | AgentSnapshotExportDialog, TeamSnapshotExportDialog | — | in-pr4 |
| `agents.common.memory-stored-as` | Memory is stored as | AgentSnapshotExportDialog, TeamSnapshotExportDialog | — | in-pr4 |
| `agents.common.plaintext` | plaintext | AgentSnapshotExportDialog, TeamSnapshotExportDialog | — | in-pr4 |
| `agents.common.team-all-memories` | Team + all memories | TeamSnapshotExportDialog | — | in-pr4 |
| `agents.common.team-core-memory` | Team + core memory | TeamSnapshotExportDialog | — | in-pr4 |
| `agents.common.team-only` | Team only | TeamSnapshotExportDialog | — | in-pr4 |
| `agents.snapshot-export.cancel` | Cancel | AgentSnapshotExportDialog | — | in-pr4 |
| `agents.snapshot-export.export` | Export | AgentSnapshotExportDialog | — | in-pr4 |
| `agents.team-snapshot-export.cancel` | Cancel | TeamSnapshotExportDialog | — | in-pr4 |
| `agents.team-snapshot-export.export` | Export | TeamSnapshotExportDialog | — | in-pr4 |

| `channel-templates.apply.failed-count_one` | {{count}} agent from the template could not be created | useApplyTemplate | — | in-pr4 |
| `channel-templates.apply.failed-count_other` | {{count}} agents from the template could not be created | useApplyTemplate | — | in-pr4 |
| `chat.header.copy-failed` | Failed to copy channel name | ChatHeader | — | in-pr4 |
| `chat.header.copy-name-aria` | Copy channel name: {{name}} | ChatHeader | — | in-pr4 |
| `chat.header.copy-success` | Channel name copied | ChatHeader | — | in-pr4 |
| `huddle.add-agent-dialog.adding` | Adding {{name}} | AddAgentDialog | — | in-pr4 |
| `huddle.add-agent-dialog.all-added` | All available agents are already in this huddle. | AddAgentDialog | — | in-pr4 |
| `huddle.add-agent-dialog.loading` | Loading agents… | AddAgentDialog | — | in-pr4 |
| `huddle.participants.remove-agent-label` | Remove from huddle | AgentVoiceMenu | — | in-pr4 |
| `local-archive.kinds.buzz-native-deletions` | Buzz-native deletions (kind 9005) | local-archive surface | — | in-pr4 |
| `local-archive.kinds.event-deletions` | Event deletions (kind 5) | local-archive surface | — | in-pr4 |
| `local-archive.kinds.forum-comments` | Forum comments (kind 45003) | local-archive surface | — | in-pr4 |
| `local-archive.kinds.forum-posts` | Forum posts (kind 45001) | local-archive surface | — | in-pr4 |
| `local-archive.kinds.generic` | Kind {{kind}} | local-archive surface | — | in-pr4 |
| `local-archive.kinds.huddle-ended` | Huddle ended | local-archive surface | — | in-pr4 |
| `local-archive.kinds.huddle-events` | Huddle events | local-archive surface | — | in-pr4 |
| `local-archive.kinds.huddle-started` | Huddle started | local-archive surface | — | in-pr4 |
| `local-archive.kinds.message-diffs` | Message diffs (kind 40008) | local-archive surface | — | in-pr4 |
| `local-archive.kinds.message-edits` | Message edits (kind 40003) | local-archive surface | — | in-pr4 |
| `local-archive.kinds.messages` | Messages & posts | local-archive surface | — | in-pr4 |
| `local-archive.kinds.participant-joined` | Participant joined | local-archive surface | — | in-pr4 |
| `local-archive.kinds.participant-left` | Participant left | local-archive surface | — | in-pr4 |
| `local-archive.kinds.reactions` | Reactions (kind 7) | local-archive surface | — | in-pr4 |
| `local-archive.kinds.reactions-edits-deletions` | Reactions, edits & deletions | local-archive surface | — | in-pr4 |
| `local-archive.kinds.stream-messages` | Stream messages (kind 9) | local-archive surface | — | in-pr4 |
| `local-archive.kinds.stream-messages-v2` | Stream messages v2 (kind 40002) | local-archive surface | — | in-pr4 |
| `local-archive.kinds.system-messages` | System messages | local-archive surface | — | in-pr4 |
| `local-archive.kinds.system-messages-40099` | System messages (kind 40099) | local-archive surface | — | in-pr4 |
| `local-archive.settings-card.add-subscription` | Add channel subscription | LocalArchiveSettingsCard | — | in-pr4 |
| `local-archive.settings-card.channel` | Channel | LocalArchiveSettingsCard | — | in-pr4 |
| `local-archive.settings-card.channel-subs` | Channel subscriptions | LocalArchiveSettingsCard | — | in-pr4 |
| `local-archive.settings-card.channel-subs-count` | Channel subscriptions ({{count}}) | LocalArchiveSettingsCard | — | in-pr4 |
| `local-archive.settings-card.create-failed` | Failed to create subscription. | LocalArchiveSettingsCard | — | in-pr4 |
| `local-archive.settings-card.custom-kinds` | Advanced: custom kinds | LocalArchiveSettingsCard | — | in-pr4 |
| `local-archive.settings-card.custom-kinds-help` | Space- or comma-separated non-negative integers. Kinds already in the checklist above are ignored. | LocalArchiveSettingsCard | — | in-pr4 |
| `local-archive.settings-card.description` | Save copies of relay messages to a local SQLite database in your Buzz nest. Events are re-verified against the relay at archive time. | LocalArchiveSettingsCard | — | in-pr4 |
| `local-archive.settings-card.event-types` | Event types | LocalArchiveSettingsCard | — | in-pr4 |
| `local-archive.settings-card.invalid-tokens` | Invalid tokens (ignored): | LocalArchiveSettingsCard | — | in-pr4 |
| `local-archive.settings-card.no-subs` | No channel subscriptions yet. Add one below. | LocalArchiveSettingsCard | — | in-pr4 |
| `local-archive.settings-card.observer-description` | Saves kind {{kind}} observer frames addressed to your pubkey. These are ephemeral — not stored by the relay — so local archiving is the only way to retain them. | LocalArchiveSettingsCard | — | in-pr4 |
| `local-archive.settings-card.observer-disabled` | Observer feed archive disabled. | LocalArchiveSettingsCard | — | in-pr4 |
| `local-archive.settings-card.observer-enabled` | Observer feed archive enabled. | LocalArchiveSettingsCard | — | in-pr4 |
| `local-archive.settings-card.observer-feed` | Agent observer feed | LocalArchiveSettingsCard | — | in-pr4 |
| `local-archive.settings-card.observer-toggle` | Archive my agents' observer frames | LocalArchiveSettingsCard | — | in-pr4 |
| `local-archive.settings-card.observer-update-failed` | Failed to update observer archive. | LocalArchiveSettingsCard | — | in-pr4 |
| `local-archive.settings-card.remove-aria` | Remove archive subscription for {{name}} | LocalArchiveSettingsCard | — | in-pr4 |
| `local-archive.settings-card.remove-failed` | Failed to remove subscription. | LocalArchiveSettingsCard | — | in-pr4 |
| `local-archive.settings-card.select-channel` | Select a channel… | LocalArchiveSettingsCard | — | in-pr4 |
| `local-archive.settings-card.subscribe` | Subscribe to a channel | LocalArchiveSettingsCard | — | in-pr4 |
| `local-archive.settings-card.subscribe-description` | Choose a channel and select which event types to archive. | LocalArchiveSettingsCard | — | in-pr4 |
| `local-archive.settings-card.subscription-created` | Archive subscription created. | LocalArchiveSettingsCard | — | in-pr4 |
| `local-archive.settings-card.subscription-removed` | Archive subscription removed. | LocalArchiveSettingsCard | — | in-pr4 |
| `local-archive.settings-card.title` | Local archive | LocalArchiveSettingsCard | — | in-pr4 |
| `local-archive.settings-card.turn-metrics` | Agent turn metrics | LocalArchiveSettingsCard | — | in-pr4 |
| `local-archive.settings-card.turn-metrics-description` | Saves kind {{kind}} turn-metric events addressed to your pubkey. Stored as plaintext in your local archive so token-usage calculators can read them directly. | LocalArchiveSettingsCard | — | in-pr4 |
| `local-archive.settings-card.turn-metrics-disabled` | Agent turn metric archive disabled. | LocalArchiveSettingsCard | — | in-pr4 |
| `local-archive.settings-card.turn-metrics-enabled` | Agent turn metric archive enabled. | LocalArchiveSettingsCard | — | in-pr4 |
| `local-archive.settings-card.turn-metrics-toggle` | Archive my agents' turn metrics | LocalArchiveSettingsCard | — | in-pr4 |
| `local-archive.settings-card.turn-metrics-update-failed` | Failed to update agent metric archive. | LocalArchiveSettingsCard | — | in-pr4 |
| `pulse.agent-activity.status-aria` | Agent {{status}} | AgentActivityCard | — | in-pr4 |
| `shared.feature-enabled.preview-hint` | {{name}} is a preview feature. Enable it in Settings → Experiments to surface it in your sidebar. | useFeatureEnabled | — | in-pr4 |
| `shared.reconnect.still-trying` | Still trying to reconnect — check your network. | useReconnectRelay | — | in-pr4 |

| `projects.add-repo-dialog.access-channel` | Access channel | AddProjectRepositoryDialog | — | in-pr4 |
| `projects.add-repo-dialog.access-hint` | Members of this channel can access the repository. | AddProjectRepositoryDialog | — | in-pr4 |
| `projects.add-repo-dialog.adding` | Adding... | AddProjectRepositoryDialog | — | in-pr4 |
| `projects.add-repo-dialog.choose-project` | Choose a project for this repository. | AddProjectRepositoryDialog | — | in-pr4 |
| `projects.add-repo-dialog.clone-url` | Clone URL | AddProjectRepositoryDialog | — | in-pr4 |
| `projects.add-repo-dialog.create-repo-failed` | Failed to create the repository. | useAddProjectRepository | — | in-pr4 |
| `projects.add-repo-dialog.description-project` | Add another repository to {{name}}. | AddProjectRepositoryDialog | — | in-pr4 |
| `projects.agent-chat-panel.clear-aria` | Clear project agent chat | ProjectAgentChatPanel | — | in-pr4 |
| `projects.agent-chat-panel.empty-body` | Start a conversation with the project agent. | ProjectAgentChatPanel | — | in-pr4 |
| `projects.agent-chat-panel.empty-body-home` | The project agent will build it out from this channel. | ProjectAgentChatPanel | — | in-pr4 |
| `projects.agent-chat-panel.empty-title` | Ask about this page | ProjectAgentChatPanel | — | in-pr4 |
| `projects.agent-chat-panel.empty-title-home` | Explain what this project should be | ProjectAgentChatPanel | — | in-pr4 |
| `projects.agent-context-preview.disclaimer` | This exact text is appended to your message before it is signed and sent. Quoted values are untrusted workspace metadata — Buzz does not verify or rewrite them. | AgentContextPayloadPreview | — | in-pr4 |
| `projects.agent-context-preview.tooltip` | Preview the exact context appended to your message | AgentContextPayloadPreview | — | in-pr4 |
| `projects.agent-context-strip.close-chat` | Close agent chat | ProjectAgentContextStrip | — | in-pr4 |
| `projects.attach-repo-dialog.all-attached` | Every available repository is already in this project. | AttachProjectRepositoryDialog | — | in-pr4 |
| `projects.attach-repo-dialog.description` | Choose an existing repository to add to {{name}}. | AttachProjectRepositoryDialog | — | in-pr4 |
| `projects.attach-repo-dialog.title` | Add existing repository | AttachProjectRepositoryDialog | — | in-pr4 |
| `projects.author-identity.role-agent` | Agent | ProjectAuthorIdentity | — | in-pr4 |
| `projects.author-identity.role-person` | Person | ProjectAuthorIdentity | — | in-pr4 |
| `projects.branch-dialogs.branch-name` | Branch name | ProjectBranchDialogs | — | in-pr4 |
| `projects.branch-dialogs.choose-branch-first` | Choose a branch first. | projectBranches | — | in-pr4 |
| `projects.branch-dialogs.choose-remote` | Choose a remote branch. | branchMutations | — | in-pr4 |
| `projects.branch-dialogs.close-review-first` | Close the branch's review before deleting it. | projectBranches | — | in-pr4 |
| `projects.branch-dialogs.create-branch` | Create branch | ProjectBranchDialogs | — | in-pr4 |
| `projects.branch-dialogs.create-description` | Create a remote branch from {{branch}}. | ProjectBranchDialogs | — | in-pr4 |
| `projects.branch-dialogs.create-description-commit` | Create a remote branch from {{branch}} at {{commit}}. | ProjectBranchDialogs | — | in-pr4 |
| `projects.branch-dialogs.create-first-commit` | Create the repository's first commit before creating another branch. | projectBranches | — | in-pr4 |
| `projects.branch-dialogs.default-undeletable` | The repository's default branch cannot be deleted. | projectBranches | — | in-pr4 |
| `projects.branch-dialogs.delete-branch` | Delete branch | ProjectBranchDialogs | — | in-pr4 |
| `projects.branch-dialogs.delete-description` | Delete the remote branch {{branch}}. This cannot be undone and may be rejected by repository protection rules. | ProjectBranchDialogs | — | in-pr4 |
| `projects.branch-dialogs.delete-question` | Delete branch? | ProjectBranchDialogs | — | in-pr4 |
| `projects.branch-dialogs.published-only-delete` | Only a published remote branch can be deleted. | projectBranches | — | in-pr4 |
| `projects.branch-dialogs.push-first-local` | Push the first local commit to {{branch}} before creating another branch. | projectBranches | — | in-pr4 |
| `projects.branch-dialogs.refresh-hint` | Refresh the repository before creating a branch. | ProjectBranchDialogs | — | in-pr4 |
| `projects.browser-dialog.create-hint` | Configure its repository and access options | ProjectBrowserDialog | — | in-pr4 |
| `projects.browser-dialog.create-query` | Create “{{query}}” | ProjectBrowserDialog | — | in-pr4 |
| `projects.browser-dialog.empty-body` | Try another search or create a new project. | ProjectBrowserDialog | — | in-pr4 |
| `projects.browser-dialog.empty-title` | No projects found | ProjectBrowserDialog | — | in-pr4 |
| `projects.browser-dialog.search-aria` | Search projects | ProjectBrowserDialog | — | in-pr4 |
| `projects.browser-dialog.search-placeholder` | Search or create a project | ProjectBrowserDialog | — | in-pr4 |
| `projects.browser-dialog.title` | Add a project | ProjectBrowserDialog | — | in-pr4 |
| `projects.category-create.add-stream` | Add another stream to {{name}}. | ProjectsCategoryCreateDialogs | — | in-pr4 |
| `projects.category-create.choose-project` | Choose a project for this channel. | ProjectsCategoryCreateDialogs | — | in-pr4 |
| `projects.category-create.choose-project-error` | Choose a project. | ProjectsCategoryCreateDialogs | — | in-pr4 |
| `projects.channel-home.channel-missing` | This project's channel could not be found. | ProjectChannelHome | — | in-pr4 |
| `projects.channel-home.expand` | Open in repository | ProjectChannelHome | — | in-pr4 |
| `projects.channel-home.expand-tab` | Open {{title}} in repository | ProjectChannelHome | — | in-pr4 |
| `projects.channel-home.hide` | Hide {{name}} | ProjectChannelHome | — | in-pr4 |
| `projects.channel-home.show` | Show {{name}} | ProjectChannelHome | — | in-pr4 |
| `projects.channel-management.description` | Add another stream to this project. A template can keep the same canvas and agents. | ProjectChannelManagement | — | in-pr4 |
| `projects.channel-management.owner-only` | Only the project owner can add channels | ProjectChannelManagement | — | in-pr4 |
| `projects.channel-request-dialog.create-failed` | Failed to create the project channel. | useProjectChannelRequests | — | in-pr4 |
| `projects.channel-request-dialog.lifetime` | Lifetime | ProjectChannelRequestDialog | — | in-pr4 |
| `projects.channel-request-dialog.owner-only` | Only the project owner can approve this channel. | useProjectChannelRequests | — | in-pr4 |
| `projects.channel-request-dialog.requested` | Your agent requested a new channel in {{project}}. Review the details before creating it. | ProjectChannelRequestDialog | — | in-pr4 |
| `projects.channel-request-dialog.template-missing` | Channel template "{{name}}" is not available on this device. | useProjectChannelRequests | — | in-pr4 |
| `projects.channel-request-dialog.temporary` | Temporary · {{duration}}. Cleans up automatically after that period of inactivity. | ProjectChannelRequestDialog | — | in-pr4 |
| `projects.channel-request-dialog.this-project` | this project | ProjectChannelRequestDialog | — | in-pr4 |
| `projects.channel-request-dialog.title` | Create project channel? | ProjectChannelRequestDialog | — | in-pr4 |
| `projects.channels-list.channel-unavailable` | Channel unavailable | ProjectsChannelsList | — | in-pr4 |
| `projects.channels-list.details-unavailable` | Channel details are unavailable | ProjectsChannelsList | — | in-pr4 |
| `projects.channels-list.empty-description` | Link a discussion channel to a project or repository and it will appear here. | ProjectsChannelsList | — | in-pr4 |
| `projects.channels-list.empty-title` | No project channels yet | ProjectsChannelsList | — | in-pr4 |
| `projects.channels-list.loading` | Loading project channels | ProjectsChannelsList | — | in-pr4 |
| `projects.channels-list.no-matching-title` | No matching channels | ProjectsChannelsList | — | in-pr4 |
| `projects.channels-list.open-unavailable` | Open unavailable channel | ProjectsChannelsList | — | in-pr4 |
| `projects.commit-copy-button.copy-hash` | Copy commit hash | ProjectCommitCopyButton | — | in-pr4 |
| `projects.commit-detail-panel.changes` | Changes | ProjectCommitDetailPanel | — | in-pr4 |
| `projects.commit-detail-panel.commit` | Commit | ProjectCommitDetailPanel | — | in-pr4 |
| `projects.commit-detail-panel.committed` | Committed | ProjectCommitDetailPanel | — | in-pr4 |
| `projects.commit-detail-panel.copy-link` | Copy commit link | ProjectCommitDetailPanel | — | in-pr4 |
| `projects.commit-detail-panel.date` | Date | ProjectCommitDetailPanel | — | in-pr4 |
| `projects.conversation-panel.load-failed` | This conversation could not be loaded. | ProjectConversationPanel | — | in-pr4 |
| `projects.create-project-form.agent-aria` | Coding agent: {{value}} | CreateProjectFormSettings | — | in-pr4 |
| `projects.create-project-form.back-to-projects` | Back to projects | CreateProjectFormContent | — | in-pr4 |
| `projects.create-project-form.coding-agent` | Coding agent | CreateProjectFormSettings | — | in-pr4 |
| `projects.create-project-form.create-failed` | Failed to create project. | CreateProjectFormContent | — | in-pr4 |
| `projects.create-project-form.description-placeholder` | What this project should become | CreateProjectFormContent | — | in-pr4 |
| `projects.create-project-form.header-subtitle` | A project starts as a channel with a repository. People in the channel can talk, clone, and open tasks here. | CreateProjectFormContent | — | in-pr4 |
| `projects.create-project-form.listed` | Listed | CreateProjectFormSettings | — | in-pr4 |
| `projects.create-project-form.listing-aria` | Project list: {{value}} | CreateProjectFormSettings | — | in-pr4 |
| `projects.create-project-form.no-runtimes` | No agent runtimes are available. Install a runtime to add agents. | useCreateProjectFormSettings | — | in-pr4 |
| `projects.create-project-form.project-list` | Project list | CreateProjectFormSettings | — | in-pr4 |
| `projects.create-project-form.team-aria` | Team: {{value}} | CreateProjectFormSettings | — | in-pr4 |
| `projects.create-project-form.template-aria` | Template: {{value}} | CreateProjectFormSettings | — | in-pr4 |
| `projects.create-project-form.template-home-default` | Project home by default | CreateProjectFormSettings | — | in-pr4 |
| `projects.create-project-form.title` | Create a new project | CreateProjectFormContent, ProjectBrowserDialog | — | in-pr4 |
| `projects.create-project-form.unlisted` | Unlisted | CreateProjectFormSettings | — | in-pr4 |
| `projects.create-project.canvas-warning` | Project created, but its project-home canvas could not be added. | useCreateProject | — | in-pr4 |
| `projects.create-review-dialog.base` | Base | CreatePullRequestDialog | — | in-pr4 |
| `projects.create-review-dialog.body-placeholder` | Add context for reviewers | CreatePullRequestDialog | — | in-pr4 |
| `projects.create-review-dialog.branches-must-differ` | The base and compare branches must be different. | CreatePullRequestDialog | — | in-pr4 |
| `projects.create-review-dialog.choose-base` | Choose a base branch. | CreatePullRequestDialog | — | in-pr4 |
| `projects.create-review-dialog.choose-branches` | Choose a repository and branches to compare. | CreatePullRequestDialog | — | in-pr4 |
| `projects.create-review-dialog.choose-compare` | Choose a compare branch. | CreatePullRequestDialog | — | in-pr4 |
| `projects.create-review-dialog.choose-repository` | Choose a repository. | CreatePullRequestDialog | — | in-pr4 |
| `projects.create-review-dialog.compare` | Compare | CreatePullRequestDialog | — | in-pr4 |
| `projects.create-review-dialog.compare-must-push` | The compare branch must be pushed before opening a review. | CreatePullRequestDialog | — | in-pr4 |
| `projects.create-review-dialog.created` | Review created. | CreatePullRequestDialog | — | in-pr4 |
| `projects.create-review-dialog.description` | {{name}}: {{source}} → {{target}} | CreatePullRequestDialog | — | in-pr4 |
| `projects.create-review-dialog.description-commit` | {{name}}: {{source}} → {{target}} at {{commit}} | CreatePullRequestDialog | — | in-pr4 |
| `projects.create-review-dialog.incomplete` | Review branches are incomplete. | CreatePullRequestDialog | — | in-pr4 |
| `projects.create-review-dialog.open-review-exists` | An open review already compares these branches. | CreatePullRequestDialog | — | in-pr4 |
| `projects.create-review-dialog.select-branch` | Select branch | CreatePullRequestDialog | — | in-pr4 |
| `projects.create-review-dialog.title` | Open a review | CreatePullRequestDialog | — | in-pr4 |
| `projects.create-review-dialog.title-placeholder` | Describe the change | CreatePullRequestDialog | — | in-pr4 |
| `projects.create-task-dialog.choose-repository` | Choose a repository for this task. | CreateProjectIssueDialog | — | in-pr4 |
| `projects.create-task-dialog.created` | Task created. | CreateProjectIssueDialog, ProjectDetailScreen | — | in-pr4 |
| `projects.create-task-dialog.description-in` | Create a task in {{name}} | CreateIssueDialog, CreateProjectIssueDialog | — | in-pr4 |
| `projects.create-task-dialog.title` | Create a task | CreateIssueDialog, CreateProjectIssueDialog | — | in-pr4 |
| `projects.detail-feed-panels.contributors-empty-body` | Contributors appear after signed project or repository activity. | ProjectDetailFeedPanels | — | in-pr4 |
| `projects.detail-feed-panels.contributors-empty-title` | No contributors yet | ProjectDetailFeedPanels | — | in-pr4 |
| `projects.detail-feed-panels.loading` | Loading activity | ProjectDetailFeedPanels | — | in-pr4 |
| `projects.detail-feed-panels.no-git-commits` | No git commits | ProjectDetailFeedPanels | — | in-pr4 |
| `projects.detail-feed-panels.no-linked-reviews` | No linked reviews | ProjectDetailFeedPanels | — | in-pr4 |
| `projects.detail-feed-panels.no-linked-tasks` | No linked tasks | ProjectDetailFeedPanels | — | in-pr4 |
| `projects.detail-feed-panels.role-agent` | Agent contributor | ProjectDetailFeedPanels | — | in-pr4 |
| `projects.detail-feed-panels.role-buzz` | Buzz contributor | ProjectDetailFeedPanels | — | in-pr4 |
| `projects.detail-feed-panels.role-git` | Git contributor | ProjectDetailFeedPanels | — | in-pr4 |
| `projects.detail-feed-panels.role-unverified` | {{name}} · unverified match | ProjectDetailFeedPanels | — | in-pr4 |
| `projects.detail-screen.create-branch-tooltip` | Create a remote branch | ProjectDetailScreen | — | in-pr4 |
| `projects.detail-screen.delete-branch-tooltip` | Delete this remote branch | ProjectDetailScreen | — | in-pr4 |
| `projects.detail-screen.fetch-tooltip` | Check for remote changes | ProjectDetailScreen | — | in-pr4 |
| `projects.detail-screen.local` | Local | ProjectDetailScreen | — | in-pr4 |
| `projects.detail-screen.local-checking` | Local checking | ProjectDetailScreen | — | in-pr4 |
| `projects.detail-screen.local-missing` | Local missing | ProjectDetailScreen | — | in-pr4 |
| `projects.detail-screen.pull-failed` | Failed to pull repository | ProjectDetailScreen | — | in-pr4 |
| `projects.detail-screen.push-failed` | Failed to push repository | ProjectDetailScreen | — | in-pr4 |
| `projects.detail-screen.push-review-updated` | {{message}} Review updated. | ProjectDetailScreen | — | in-pr4 |
| `projects.detail-screen.refreshed` | Remote state refreshed. | ProjectDetailScreen | — | in-pr4 |
| `projects.detail-screen.review-current` | Review is already current. | ProjectDetailScreen | — | in-pr4 |
| `projects.detail-screen.review-update-failed` | Failed to update review | ProjectDetailScreen | — | in-pr4 |
| `projects.detail-screen.review-updated` | Review updated. | ProjectDetailScreen | — | in-pr4 |
| `projects.detail-unavailable.back-to-projects` | Back to Projects | ProjectDetailUnavailableState | — | in-pr4 |
| `projects.detail-unavailable.load-error` | Failed to load project | ProjectDetailUnavailableState | — | in-pr4 |
| `projects.detail-unavailable.no-repositories` | This project does not have any available repositories yet. | ProjectDetailUnavailableState | — | in-pr4 |
| `projects.detail-unavailable.not-found` | This project could not be found. | ProjectDetailUnavailableState | — | in-pr4 |
| `projects.discussion-channels.latest-note` | Showing the latest {{limit}} mentions; totals may be higher. | DiscussionChannels | — | in-pr4 |
| `projects.discussion-channels.loading` | Loading channel discussions | DiscussionChannels | — | in-pr4 |
| `projects.discussion-channels.open-channel-aria` | Open channel #{{name}} | DiscussionChannels | — | in-pr4 |
| `projects.discussion-channels.open-channel-title` | Open #{{name}} | DiscussionChannels, ProjectConversationPanel, ProjectsChannelsList | — | in-pr4 |
| `projects.discussion-channels.open-conversation-aria` | Open conversation in #{{name}} | DiscussionChannels | — | in-pr4 |
| `projects.discussion-channels.open-latest-title` | Open the latest conversation in #{{name}} | DiscussionChannels | — | in-pr4 |
| `projects.discussion-channels.panel-empty-description` | Paste this repository, review, or task link in a channel and it will appear here. | DiscussionChannels | — | in-pr4 |
| `projects.discussion-channels.panel-empty-title` | No linked channels yet | DiscussionChannels | — | in-pr4 |
| `projects.discussion-channels.related-title` | Related Conversations | DiscussionChannels | — | in-pr4 |
| `projects.discussion-channels.show-more_one` | Show {{count}} more conversation | DiscussionChannels | — | in-pr4 |
| `projects.discussion-channels.show-more_other` | Show {{count}} more conversations | DiscussionChannels | — | in-pr4 |
| `projects.discussion-channels.truncated-note` | Showing mentions from the 500 most recent search results. | DiscussionChannels | — | in-pr4 |
| `projects.home-codebase-panel.attach-hint` | Attach a repository to browse the file tree beside this channel. | ProjectHomeCodebasePanel | — | in-pr4 |
| `projects.home-commits-panel.degraded-multiple` | Showing commits from {{loaded}} of {{total}} repositories. {{failed}} repositories could not be loaded. | ProjectHomeCommitsPanel | — | in-pr4 |
| `projects.home-commits-panel.degraded-single` | Showing commits from {{loaded}} of {{total}} repositories. {{failed}} repository could not be loaded. | ProjectHomeCommitsPanel | — | in-pr4 |
| `projects.home-commits-panel.loading` | Loading commits | ProjectHomeCommitsPanel | — | in-pr4 |
| `projects.home-context-panel.add-repository` | Add a repository to this project | ProjectHomeContextPanel | — | in-pr4 |
| `projects.home-context-panel.codebase` | Codebase | ProjectHomeContextPanel | — | in-pr4 |
| `projects.home-context-panel.none-yet` | None yet | ProjectHomeContextPanel | — | in-pr4 |
| `projects.issue-assignees.assign` | Assign | IssueAssigneesRow | — | in-pr4 |
| `projects.issue-assignees.assign-failed` | Failed to assign task. | IssueAssigneesRow | — | in-pr4 |
| `projects.issue-assignees.assign-hint` | Choose a person or agent to work on this task. | IssueAssigneesRow | — | in-pr4 |
| `projects.issue-assignees.assign-task` | Assign task | IssueAssigneesRow | — | in-pr4 |
| `projects.issue-assignees.assign-to-me` | Assign to me | IssueAssigneesRow | — | in-pr4 |
| `projects.issue-assignees.assigned` | Task assigned. | IssueAssigneesRow | — | in-pr4 |
| `projects.issue-assignees.assigned-label` | {{name}} — assigned | IssueAssigneesRow | — | in-pr4 |
| `projects.issue-assignees.assigned-to` | Assigned to {{name}} | IssueAssigneesRow | — | in-pr4 |
| `projects.issue-assignees.assigned-to-me` | Assigned to me | IssueAssigneesRow | — | in-pr4 |
| `projects.issue-assignees.unassign` | Unassign {{name}} | IssueAssigneesRow | — | in-pr4 |
| `projects.issue-assignees.unassign-failed` | Failed to unassign task. | IssueAssigneesRow | — | in-pr4 |
| `projects.issue-assignees.unassigned` | Task unassigned. | IssueAssigneesRow | — | in-pr4 |
| `projects.issue-comment-timeline.collapse-history` | Collapse comment history | ProjectIssueCommentTimeline | — | in-pr4 |
| `projects.issue-comment-timeline.show-earlier_one` | Show {{count}} earlier comment | ProjectIssueCommentTimeline | — | in-pr4 |
| `projects.issue-comment-timeline.show-earlier_other` | Show {{count}} earlier comments | ProjectIssueCommentTimeline | — | in-pr4 |
| `projects.issues-list.next-open` | Open task | ProjectsIssuesList | — | in-pr4 |
| `projects.issues-list.next-review` | Review task | ProjectsIssuesList | — | in-pr4 |
| `projects.issues-list.next-triage` | Triage task | ProjectsIssuesList | — | in-pr4 |
| `projects.labels.discussion-linked` | Discussion linked | projectLabels | — | in-pr4 |
| `projects.labels.discussion-none` | No discussion | projectLabels | — | in-pr4 |
| `projects.list-header.created-date` | Created date | ProjectsListHeaderBar | — | in-pr4 |
| `projects.list-header.name` | Name | ProjectsListHeaderBar | — | in-pr4 |
| `projects.list-header.recent-activity` | Recent activity | ProjectsListHeaderBar | — | in-pr4 |
| `projects.list-header.sort-projects` | Sort projects | ProjectsListHeaderBar | — | in-pr4 |
| `projects.merge-review-button.confirm-description` | Merge {{source}} into {{target}} and push the result to the repository. The remote will reject the operation if the branch changed or conflicts. | MergePullRequestButton | — | in-pr4 |
| `projects.merge-review-button.confirm-title` | Merge review? | MergePullRequestButton | — | in-pr4 |
| `projects.merge-review-button.copy-commands` | Copy commands | MergePullRequestButton | — | in-pr4 |
| `projects.merge-review-button.merge` | Merge | MergePullRequestButton | — | in-pr4 |
| `projects.merge-review-button.merge-failed` | Failed to merge review. | MergePullRequestButton | — | in-pr4 |
| `projects.merge-review-button.merge-review` | Merge review | MergePullRequestButton | — | in-pr4 |
| `projects.merge-review-button.merging` | Merging… | MergePullRequestButton | — | in-pr4 |
| `projects.merge-review-button.prepare-failed` | Failed to prepare merge recovery. | MergePullRequestButton | — | in-pr4 |
| `projects.merge-review-button.preparing` | Preparing… | MergePullRequestButton | — | in-pr4 |
| `projects.merge-review-button.publish-failed` | Failed to publish merged review status. | MergePullRequestButton | — | in-pr4 |
| `projects.merge-review-button.publish-merged-status` | Publish merged status | MergePullRequestButton | — | in-pr4 |
| `projects.merge-review-button.published` | Published merged review status. | MergePullRequestButton | — | in-pr4 |
| `projects.merge-review-button.publishing` | Publishing… | MergePullRequestButton | — | in-pr4 |
| `projects.merge-review-button.recovery-opened` | Recovery commit fetched and terminal opened. | MergePullRequestButton | — | in-pr4 |
| `projects.merge-review-button.resolve-hint` | Resolve in Terminal securely fetches the target and review commits before showing copyable commands. | MergePullRequestButton | — | in-pr4 |
| `projects.merge-review-button.resolve-in-terminal` | Resolve in Terminal | MergePullRequestButton | — | in-pr4 |
| `projects.merge-review-button.resolve-steps` | Prepare the local checkout, then switch to {{branch}} with the commands shown. After resolving and committing, push the target branch and retry the merge. | MergePullRequestButton | — | in-pr4 |
| `projects.merge-review-button.resolve-title` | Resolve conflicts in your local checkout | MergePullRequestButton | — | in-pr4 |
| `projects.meta-rail.branch` | Branch | PullRequestMetaRail | — | in-pr4 |
| `projects.origin-reference.channel-invisible` | Origin channel {{id}} is not visible to you. | ProjectOriginReference | — | in-pr4 |
| `projects.origin-reference.not-verified` | Origin is claimed by the event author and is not relay-verified. | ProjectOriginReference | — | in-pr4 |
| `projects.origin-reference.open-origin-channel-aria` | Open author-claimed origin channel #{{name}} | ProjectOriginReference | — | in-pr4 |
| `projects.origin-reference.private-omitted` | The private conversation identifier is intentionally omitted. | ProjectOriginReference | — | in-pr4 |
| `projects.origin-reference.started-from` | started from | ProjectOriginReference | — | in-pr4 |
| `projects.overview-chrome.chat-aria` | Chat with an agent about {{section}} | ProjectsOverviewChromeActions | — | in-pr4 |
| `projects.overview-chrome.context-hide` | Hide project context | ProjectRightPanelControls, ProjectsOverviewChromeActions | — | in-pr4 |
| `projects.overview-chrome.context-show` | Show project context | ProjectRightPanelControls, ProjectsOverviewChromeActions | — | in-pr4 |
| `projects.pr-panel-surface.empty-description` | Reviews opened for this repository will appear here. | PullRequestsPanelSurface | — | in-pr4 |
| `projects.pr-panel-surface.no-reviews` | No reviews yet | PullRequestsPanelSurface | — | in-pr4 |
| `projects.pull-requests-list.copy-review-link` | Copy review link | ProjectsPullRequestsList | — | in-pr4 |
| `projects.pull-requests-list.load-error` | Could not load reviews | ProjectsPullRequestsList, PullRequestsPanelSurface | — | in-pr4 |
| `projects.pull-requests-list.loading` | Loading reviews | ProjectsPullRequestsList, PullRequestsPanelSurface | — | in-pr4 |
| `projects.pull-requests-list.next-closed` | View closed | ProjectsPullRequestsList | — | in-pr4 |
| `projects.pull-requests-list.next-draft` | View draft | ProjectsPullRequestsList | — | in-pr4 |
| `projects.pull-requests-list.next-merge` | View merge | ProjectsPullRequestsList | — | in-pr4 |
| `projects.pull-requests-list.next-open` | Open review | ProjectsPullRequestsList | — | in-pr4 |
| `projects.repo-open.folder-error-title` | Couldn’t open repository folder | useProjectRepositoryOpenActions | — | in-pr4 |
| `projects.repo-open.folder-missing-description` | Buzz could not find this repository’s local checkout. | useProjectRepositoryOpenActions | — | in-pr4 |
| `projects.repo-open.folder-open-description` | Buzz could not open this checkout in your file browser. | useProjectRepositoryOpenActions | — | in-pr4 |
| `projects.repository-card.buzz-hosted` | Buzz-hosted repository | RepositoryCards | — | in-pr4 |
| `projects.repository-card.external-hosted` | Git data hosted on {{host}} | RepositoryCards | — | in-pr4 |
| `projects.repository-card.host-label` | Repository host | RepositoryCards | — | in-pr4 |
| `projects.review-decision.approve-failed` | Failed to approve review. | pullRequestReviews | — | in-pr4 |
| `projects.review-decision.no-commit` | The review has no commit to inspect. | pullRequestReviews | — | in-pr4 |
| `projects.review-decision.request-failed` | Failed to request changes. | pullRequestReviews | — | in-pr4 |
| `projects.review-decision.timeout-approving` | Timed out approving review. | pullRequestReviews | — | in-pr4 |
| `projects.review-decision.timeout-requesting` | Timed out requesting changes. | pullRequestReviews | — | in-pr4 |
| `projects.section-search.close-project-search` | Close project search | ProjectsSectionSearch | — | in-pr4 |
| `projects.section-search.close-search` | Close search | ProjectsSectionSearch | — | in-pr4 |
| `projects.section-search.search-section` | Search {{section}} | ProjectsSectionSearch | — | in-pr4 |
| `projects.selectable-group.clear-all` | Clear all {{label}} | ProjectSelectableGroup | — | in-pr4 |
| `projects.selectable-group.select-all` | Select all {{label}} | ProjectSelectableGroup | — | in-pr4 |
| `projects.selection-count-menu.links-copied` | Links copied | ProjectsSelectionCountMenu | — | in-pr4 |
| `projects.share-link.copied` | Link copied | ShareLinkButton | — | in-pr4 |
| `projects.shared.add-repo-failed` | Failed to add repository. | AddProjectRepositoryDialog | — | in-pr4 |
| `projects.shared.agent-prefix` | Agent ·  | IssueAssigneesRow | — | in-pr4 |
| `projects.shared.agent-unreachable` | Failed to reach the agent | ProjectAgentChatPanel | — | in-pr4 |
| `projects.shared.attach-repo-failed` | Failed to attach repository. | AttachProjectRepositoryDialog | — | in-pr4 |
| `projects.shared.channel-created` | Channel "#{{name}}" created. | ProjectChannelManagement, ProjectsCategoryCreateDialogs, useProjectChannelRequests | — | in-pr4 |
| `projects.shared.clear-conversation` | Clear conversation | ProjectAgentChatPanel | — | in-pr4 |
| `projects.shared.commits-project-description` | Commits pushed to this project's repositories will appear here. | ProjectDetailFeedPanels, ProjectHomeCommitsPanel | — | in-pr4 |
| `projects.shared.commits-repository-description` | Commits pushed to this repository will appear here. | ProjectDetailFeedPanels | — | in-pr4 |
| `projects.shared.could-not-load-commits` | Could not load commits | ProjectDetailFeedPanels | — | in-pr4 |
| `projects.shared.create-project-channel` | Create a project channel | ProjectChannelManagement, ProjectsCategoryCreateDialogs | — | in-pr4 |
| `projects.shared.creating` | Creating… | CreateProjectWorkItemDialog, ProjectBranchDialogs, ProjectChannelRequestDialog | — | in-pr4 |
| `projects.shared.deleting` | Deleting… | ProjectBranchDialogs | — | in-pr4 |
| `projects.shared.files-changed` | Files changed | ProjectCommitDetailPanel | — | in-pr4 |
| `projects.shared.no-agents-available` | No agents available | ProjectAgentChatPanel | — | in-pr4 |
| `projects.shared.no-commits-title` | No commits yet | ProjectDetailFeedPanels, ProjectHomeCommitsPanel | — | in-pr4 |
| `projects.shared.project` | Project | AddProjectRepositoryDialog, ProjectDetailScreen, ProjectsCategoryCreateDialogs | — | in-pr4 |
| `projects.shared.project-context` | Project context | ProjectRightPanelControls, ProjectsOverviewContextSheet, ProjectsView | — | in-pr4 |
| `projects.shared.refresh-retry` | Refresh the repository and try again. | ProjectDetailFeedPanels, PullRequestsPanelSurface | — | in-pr4 |
| `projects.shared.remote` | Remote | useProjectRepoHost | — | in-pr4 |
| `projects.shared.repository` | Repository | CreateProjectIssueDialog, CreatePullRequestDialog | — | in-pr4 |
| `projects.shared.retry` | Retry | ProjectDetailUnavailableState, ProjectRepositoryUnavailableState, ProjectsIssuesList, ProjectsPullRequestsList | — | in-pr4 |
| `projects.shared.select-item` | Select {{title}} | ProjectEntityListRow | — | in-pr4 |
| `projects.shared.title` | Title | CreateProjectWorkItemDialog | — | in-pr4 |
| `projects.shared.try-different-search` | Try a different search. | ProjectsChannelsList | — | in-pr4 |
| `projects.shared.unknown-error` | Unknown error | projectEnumeration | — | in-pr4 |
| `projects.shared.view-profile` | View {{name}}'s profile | ProjectCards | — | in-pr4 |
| `projects.submitted-context-pill.hide` | Hide sent context | ProjectAgentSubmittedContextPill | — | in-pr4 |
| `projects.submitted-context-pill.label` | Sent context | ProjectAgentSubmittedContextPill | — | in-pr4 |
| `projects.submitted-context-pill.payload-aria` | Sent project context | ProjectAgentSubmittedContextPill | — | in-pr4 |
| `projects.submitted-context-pill.show` | Show sent context | ProjectAgentSubmittedContextPill | — | in-pr4 |
| `projects.sync.publish-failed` | The review update could not be published. | repoSyncHooks | — | in-pr4 |
| `projects.work-item-table.actions` | Actions | ProjectsWorkItemTable | — | in-pr4 |
| `projects.work-item-table.replies` | Replies | ProjectsWorkItemTable | — | in-pr4 |
| `projects.work-item-table.updated` | Updated | ProjectsWorkItemTable | — | in-pr4 |
| `projects.work-items-list.retrying` | Retrying... | ProjectsIssuesList, ProjectsPullRequestsList | — | in-pr4 |
| `projects.work-items-notice.could-not-load` | Could not load {{subject}}. | ProjectsWorkItemsLoadNotice | — | in-pr4 |
| `projects.work-items-notice.missing-sections` | Missing {{sections}}. The available results are shown below. | ProjectsWorkItemsLoadNotice | — | in-pr4 |
| `projects.work-items-notice.partial-details` | Some {{subject}} details could not be loaded. | ProjectsWorkItemsLoadNotice | — | in-pr4 |
| `projects.work-items-notice.relay-failed` | The relay request failed. | ProjectsIssuesList, ProjectsPullRequestsList, ProjectsWorkItemsLoadNotice | — | in-pr4 |
| `projects.work-items-notice.section-assignments` | assignments | ProjectsWorkItemsLoadNotice | — | in-pr4 |
| `projects.work-items-notice.section-comments` | comments | ProjectsWorkItemsLoadNotice | — | in-pr4 |
| `projects.work-items-notice.section-review-updates` | review updates | ProjectsWorkItemsLoadNotice | — | in-pr4 |
| `projects.work-items-notice.section-statuses` | statuses | ProjectsWorkItemsLoadNotice | — | in-pr4 |
| `projects.work-items-notice.subject-review` | review | ProjectsWorkItemsLoadNotice | — | in-pr4 |
| `projects.work-items-notice.subject-reviews` | reviews | ProjectsWorkItemsLoadNotice | — | in-pr4 |
| `projects.work-items-notice.subject-task` | task | ProjectsWorkItemsLoadNotice | — | in-pr4 |
| `projects.work-items-notice.subject-tasks` | tasks | ProjectsWorkItemsLoadNotice | — | in-pr4 |
| `projects.workspace-sheet.back-to-commits` | Back to Commits | ProjectHomeWorkspaceSheet | — | in-pr4 |
| `projects.workspace-sheet.back-to-files` | Back to Files | ProjectHomeWorkspaceSheet | — | in-pr4 |
| `projects.workspace-sheet.back-to-reviews` | Back to Reviews | ProjectHomeWorkspaceSheet | — | in-pr4 |
| `projects.workspace-sheet.back-to-tasks` | Back to Tasks | ProjectHomeWorkspaceSheet | — | in-pr4 |

| `projects.overview-items.mine` | Mine | ProjectsOverviewItems | — | in-pr4 |
| `projects.overview-items.other-projects` | Other projects | ProjectsOverviewItems | — | in-pr4 |
| `projects.overview-items.other-repositories` | Other repositories | ProjectsOverviewItems | — | in-pr4 |
| `projects.right-panel.chat-hide` | Hide project chat | ProjectRightPanelControls | — | in-pr4 |
| `projects.right-panel.chat-show` | Show project chat | ProjectRightPanelControls | — | in-pr4 |
| `projects.right-panel.chat-title` | Project chat | ProjectRightPanelControls | — | in-pr4 |
| `projects.view.load-failed` | Failed to load projects | ProjectsView | — | in-pr4 |
| `projects.view.no-matching-reviews` | No matching reviews | ProjectsView | — | in-pr4 |
| `projects.view.no-matching-tasks` | No matching tasks | ProjectsView | — | in-pr4 |

| `forum.composer.cancel` | Cancel | ForumComposer | — | in-pr4 |
| `forum.composer.comment` | Comment | ForumComposer | — | in-pr4 |
| `forum.delete.cancel` | Cancel | DeleteConfirmDialog | — | in-pr4 |
| `forum.delete.confirm` | Delete {{label}} | DeleteConfirmDialog | — | in-pr4 |
| `forum.delete.confirm-description` | This will permanently delete this {{label}} and cannot be undone. | DeleteConfirmDialog | — | in-pr4 |
| `forum.delete.confirm-title` | Delete {{label}}? | DeleteConfirmDialog | — | in-pr4 |
| `forum.delete.menu-item` | Delete {{label}} | DeleteActionMenu | — | in-pr4 |
| `forum.thread.back-to-posts` | Back to posts | ForumThreadPanel | — | in-pr4 |
| `forum.thread.no-replies` | No replies yet. Be the first to respond. | ForumThreadPanel | — | in-pr4 |
| `forum.thread.reply-placeholder` | Reply to this post... | ForumThreadPanel | — | in-pr4 |
| `forum.view.archived` | This forum is archived. | ForumView | — | in-pr4 |
| `forum.view.composer-placeholder` | Write your post... | ForumView | — | in-pr4 |
| `forum.view.empty-description` | Start a discussion by creating the first post. | ForumView | — | in-pr4 |
| `forum.view.empty-title` | No posts yet | ForumView | — | in-pr4 |
| `forum.view.join-to-post` | Join this forum to create posts. | ForumView | — | in-pr4 |
| `forum.view.new-post` | Start a new post... | ForumView | — | in-pr4 |

| `workflows.actions-menu.aria` | Workflow actions | WorkflowActionsMenu | — | in-pr4 |
| `workflows.actions-menu.delete` | Delete | WorkflowActionsMenu | — | in-pr4 |
| `workflows.actions-menu.duplicate` | Duplicate | WorkflowActionsMenu | — | in-pr4 |
| `workflows.actions-menu.edit` | Edit | WorkflowActionsMenu | — | in-pr4 |
| `workflows.actions-menu.enable` | Enable | WorkflowActionsMenu | — | in-pr4 |
| `workflows.actions-menu.trigger` | Trigger | WorkflowActionsMenu | — | in-pr4 |
| `workflows.activation.keep-off` | Keep off | WorkflowDialog, WorkflowsView | — | in-pr4 |
| `workflows.activation.turn-on` | Turn on | WorkflowDialog, WorkflowsView | — | in-pr4 |
| `workflows.activation.turn-on-description` | Turn it on to let it run immediately, or keep it off until you’re ready. | WorkflowDialog, WorkflowsView | — | in-pr4 |
| `workflows.activation.turn-on-title` | Turn on this workflow? | WorkflowDialog, WorkflowsView | — | in-pr4 |
| `workflows.author-picker.empty` | No authors found. | WorkflowAuthorPicker | — | in-pr4 |
| `workflows.author-picker.list-aria` | Authors | WorkflowAuthorPicker | — | in-pr4 |
| `workflows.author-picker.load-error` | Couldn’t load authors. | WorkflowAuthorPicker | — | in-pr4 |
| `workflows.author-picker.load-more` | Load more authors | WorkflowAuthorPicker | — | in-pr4 |
| `workflows.author-picker.loading` | Loading authors… | WorkflowAuthorPicker | — | in-pr4 |
| `workflows.author-picker.loading-more` | Loading more… | WorkflowAuthorPicker | — | in-pr4 |
| `workflows.author-picker.retry` | Couldn’t load all authors. Retry | WorkflowAuthorPicker | — | in-pr4 |
| `workflows.author-picker.search-aria` | Search authors or paste a public key | WorkflowAuthorPicker | — | in-pr4 |
| `workflows.author-picker.search-placeholder` | Search people or paste a public key… | WorkflowAuthorPicker | — | in-pr4 |
| `workflows.card.disable` | Disable workflow | WorkflowCard | — | in-pr4 |
| `workflows.card.view` | View {{name}} | WorkflowCard | — | in-pr4 |
| `workflows.channel-combobox.aria-default` | Channel | ChannelCombobox | — | in-pr4 |
| `workflows.channel-combobox.empty` | No channels found. | ChannelCombobox | — | in-pr4 |
| `workflows.channel-combobox.empty-default` | Choose a channel | ChannelCombobox | — | in-pr4 |
| `workflows.channel-combobox.options-aria` | {{label}} options | ChannelCombobox | — | in-pr4 |
| `workflows.channel-combobox.readonly-aria` | {{label}}: {{value}}. Read only. | ChannelCombobox | — | in-pr4 |
| `workflows.channel-combobox.readonly-tooltip` | The channel can't be changed after a workflow is created. | ChannelCombobox | — | in-pr4 |
| `workflows.channel-combobox.search-aria` | Search {{label}}s | ChannelCombobox | — | in-pr4 |
| `workflows.channel-combobox.search-placeholder` | Search channels... | ChannelCombobox | — | in-pr4 |
| `workflows.channel-combobox.unavailable` | Unavailable channel | ChannelCombobox | — | in-pr4 |
| `workflows.condition.advanced` | Advanced | WorkflowMessageTextConditionEditor, WorkflowTriggerConditions | — | in-pr4 |
| `workflows.condition.advanced-expression-aria` | Advanced expression | WorkflowMessageTextConditionEditor, WorkflowTriggerConditions | — | in-pr4 |
| `workflows.condition.basic` | Basic | WorkflowMessageTextConditionEditor, WorkflowTriggerConditions | — | in-pr4 |
| `workflows.condition.match-legend` | Match | WorkflowMessageTextConditionEditor, WorkflowTriggerConditions | — | in-pr4 |
| `workflows.condition.mode-aria` | Condition editor mode | WorkflowMessageTextConditionEditor, WorkflowTriggerConditions | — | in-pr4 |
| `workflows.detail.close-aria` | Close detail panel | WorkflowDetailPanel | — | in-pr4 |
| `workflows.detail.current-step` | Current step {{number}} | WorkflowDetailPanel | — | in-pr4 |
| `workflows.detail.definition` | Definition | WorkflowDetailPanel | — | in-pr4 |
| `workflows.detail.edit` | Edit | WorkflowDetailPanel | — | in-pr4 |
| `workflows.detail.execution-trace` | Execution Trace | WorkflowDetailPanel | — | in-pr4 |
| `workflows.detail.history-error` | Failed to load run history | WorkflowDetailPanel | — | in-pr4 |
| `workflows.detail.history-error-fallback` | Run history could not be loaded. | WorkflowDetailPanel | — | in-pr4 |
| `workflows.detail.history-loading-aria` | Loading run history | WorkflowDetailPanel | — | in-pr4 |
| `workflows.detail.load-error` | Failed to load workflow | WorkflowDetailPanel | — | in-pr4 |
| `workflows.detail.no-runs` | No runs yet. | WorkflowDetailPanel | — | in-pr4 |
| `workflows.detail.reason-approval-denied` | Approval was denied. | WorkflowDetailPanel | — | in-pr4 |
| `workflows.detail.reason-approval-expired` | Approval expired before the workflow could continue. | WorkflowDetailPanel | — | in-pr4 |
| `workflows.detail.reason-external-outcome-unknown` | The external action may have completed, but its outcome could not be confirmed. | WorkflowDetailPanel | — | in-pr4 |
| `workflows.detail.reason-run-interrupted` | The run was interrupted before it could finish. | WorkflowDetailPanel | — | in-pr4 |
| `workflows.detail.refreshing-approvals` | Refreshing approvals... | WorkflowDetailPanel | — | in-pr4 |
| `workflows.detail.run-created` | Run created | WorkflowDetailPanel | — | in-pr4 |
| `workflows.detail.run-failed-generic` | Run failed ({{code}}). | WorkflowDetailPanel | — | in-pr4 |
| `workflows.detail.run-history` | Run History | WorkflowDetailPanel | — | in-pr4 |
| `workflows.detail.trigger` | Trigger | WorkflowDetailPanel | — | in-pr4 |
| `workflows.detail.trigger-error` | Failed to trigger workflow | WorkflowDetailPanel | — | in-pr4 |
| `workflows.detail.trigger-error-fallback` | The relay did not create a workflow run. | WorkflowDetailPanel | — | in-pr4 |
| `workflows.detail.triggering` | Triggering... | WorkflowDetailPanel | — | in-pr4 |
| `workflows.detail.waiting-trace` | Waiting for its persisted trace… | WorkflowDetailPanel | — | in-pr4 |
| `workflows.dialog.add-first-step-aria` | Add first step | WorkflowDialog | — | in-pr4 |
| `workflows.dialog.back` | Back | WorkflowDialog | — | in-pr4 |
| `workflows.dialog.cancel` | Cancel | WorkflowDialog | — | in-pr4 |
| `workflows.dialog.description-create` | Automate actions when something happens in a channel. | WorkflowDialog | — | in-pr4 |
| `workflows.dialog.description-duplicate` | Copy this workflow and adjust its details. | WorkflowDialog | — | in-pr4 |
| `workflows.dialog.description-edit` | Update when this workflow runs and what it does. | WorkflowDialog | — | in-pr4 |
| `workflows.dialog.discard` | Discard changes | WorkflowDialog | — | in-pr4 |
| `workflows.dialog.discard-description` | Your unsaved workflow changes will be lost. | WorkflowDialog | — | in-pr4 |
| `workflows.dialog.discard-title` | Discard changes? | WorkflowDialog | — | in-pr4 |
| `workflows.dialog.edit-name` | Edit workflow name | WorkflowDialog | — | in-pr4 |
| `workflows.dialog.editor-mode-aria` | Workflow editor mode | WorkflowDialog | — | in-pr4 |
| `workflows.dialog.form-tab` | Form | WorkflowDialog | — | in-pr4 |
| `workflows.dialog.generating-name` | Generating name… | WorkflowDialog | — | in-pr4 |
| `workflows.dialog.go-back` | Go back | WorkflowDialog | — | in-pr4 |
| `workflows.dialog.history` | Run history | WorkflowDialog | — | in-pr4 |
| `workflows.dialog.keep-editing` | Keep editing | WorkflowDialog | — | in-pr4 |
| `workflows.dialog.name-aria` | Workflow name | WorkflowDialog | — | in-pr4 |
| `workflows.dialog.no-channels` | Join or create a channel before adding a workflow. | WorkflowDialog | — | in-pr4 |
| `workflows.dialog.pending-create` | Creating… | WorkflowDialog | — | in-pr4 |
| `workflows.dialog.pending-edit` | Saving… | WorkflowDialog | — | in-pr4 |
| `workflows.dialog.save-name` | Save workflow name | WorkflowDialog | — | in-pr4 |
| `workflows.dialog.secret-description` | This private webhook secret cannot be recovered. Copy and store it before continuing, or explicitly leave it behind. | WorkflowDialog | — | in-pr4 |
| `workflows.dialog.secret-title` | Continue without this secret? | WorkflowDialog | — | in-pr4 |
| `workflows.dialog.section-workflow` | Workflow | WorkflowDialog | — | in-pr4 |
| `workflows.dialog.submit-create` | Create workflow | WorkflowDialog | — | in-pr4 |
| `workflows.dialog.submit-duplicate` | Create copy | WorkflowDialog | — | in-pr4 |
| `workflows.dialog.submit-edit` | Save changes | WorkflowDialog | — | in-pr4 |
| `workflows.dialog.title-create` | Create workflow | WorkflowDialog | — | in-pr4 |
| `workflows.dialog.title-duplicate` | Duplicate workflow | WorkflowDialog | — | in-pr4 |
| `workflows.dialog.title-edit` | Edit workflow | WorkflowDialog | — | in-pr4 |
| `workflows.dialog.untitled-name` | Untitled workflow | WorkflowDialog | — | in-pr4 |
| `workflows.dialog.webhook-url-error` | Could not load the webhook URL | WorkflowDialog | — | in-pr4 |
| `workflows.headers.add` | Add header | WorkflowWebhookHeadersEditor | — | in-pr4 |
| `workflows.headers.empty` | No custom headers. | WorkflowWebhookHeadersEditor | — | in-pr4 |
| `workflows.headers.name-placeholder` | Header name | WorkflowWebhookHeadersEditor | — | in-pr4 |
| `workflows.headers.remove-aria` | Remove header | WorkflowWebhookHeadersEditor | — | in-pr4 |
| `workflows.headers.title` | Headers (optional) | WorkflowWebhookHeadersEditor | — | in-pr4 |
| `workflows.headers.value-placeholder` | Header value | WorkflowWebhookHeadersEditor | — | in-pr4 |
| `workflows.message-picker.channel-placeholder` | Choose a channel first | WorkflowMessagePicker | — | in-pr4 |
| `workflows.message-picker.empty` | No messages yet. | WorkflowMessagePicker | — | in-pr4 |
| `workflows.message-picker.invalid-direct` | That message is not available in this channel. | WorkflowMessagePicker | — | in-pr4 |
| `workflows.message-picker.list-aria` | Messages | WorkflowMessagePicker | — | in-pr4 |
| `workflows.message-picker.load-error` | Couldn’t load messages. | WorkflowMessagePicker | — | in-pr4 |
| `workflows.message-picker.load-older` | Load older messages | WorkflowMessagePicker | — | in-pr4 |
| `workflows.message-picker.loading` | Loading messages… | WorkflowMessagePicker | — | in-pr4 |
| `workflows.message-picker.loading-aria` | Loading messages | WorkflowMessagePicker | — | in-pr4 |
| `workflows.message-picker.loading-older` | Loading older messages… | WorkflowMessagePicker | — | in-pr4 |
| `workflows.message-picker.no-body` | No message body | WorkflowMessagePicker | — | in-pr4 |
| `workflows.message-picker.retry` | Couldn’t load all messages. Retry | WorkflowMessagePicker | — | in-pr4 |
| `workflows.message-picker.search-aria` | Search messages or paste a message ID | WorkflowMessagePicker | — | in-pr4 |
| `workflows.message-picker.search-empty` | No messages found. | WorkflowMessagePicker | — | in-pr4 |
| `workflows.message-picker.search-placeholder` | Search messages or paste a message ID… | WorkflowMessagePicker | — | in-pr4 |
| `workflows.message-picker.selected-label` | Selected message | WorkflowMessagePicker | — | in-pr4 |
| `workflows.rich-trigger.loading-author-aria` | Loading author | WorkflowRichTriggerDescription | — | in-pr4 |
| `workflows.step-card.approval-message-placeholder` | Approval request message | WorkflowStepCard | — | in-pr4 |
| `workflows.step-card.backend-note-request-approval` | Backend note: approval gates still stop runs with WF-08; approval records are not persisted yet. | WorkflowStepCard | — | in-pr4 |
| `workflows.step-card.backend-note-send-dm` | Backend note: `send_dm` is not executed yet, so runs fail at this step. | WorkflowStepCard | — | in-pr4 |
| `workflows.step-card.backend-note-set-channel-topic` | Backend note: `set_channel_topic` is not executed yet, so runs fail at this step. | WorkflowStepCard | — | in-pr4 |
| `workflows.step-card.channel-defaults` | Defaults to the channel that triggered the workflow. Webhook and manual triggers require a channel. | WorkflowStepCard | — | in-pr4 |
| `workflows.step-card.channel-hint` | Messages post to the workflow channel selected above. | WorkflowStepCard | — | in-pr4 |
| `workflows.step-card.channel-override` | Channel override (optional) | WorkflowStepCard | — | in-pr4 |
| `workflows.step-card.details` | Step details | WorkflowStepCard | — | in-pr4 |
| `workflows.step-card.dm-content-placeholder` | DM content | WorkflowStepCard | — | in-pr4 |
| `workflows.step-card.duration-placeholder` | e.g. 5s, 1m, 1h | WorkflowStepCard | — | in-pr4 |
| `workflows.step-card.emoji` | Emoji | WorkflowStepCard | — | in-pr4 |
| `workflows.step-card.emoji-name-placeholder` | e.g. thumbsup | WorkflowStepCard | — | in-pr4 |
| `workflows.step-card.endpoint-url` | Endpoint URL | WorkflowStepCard | — | in-pr4 |
| `workflows.step-card.from-approver` | From (approver) | WorkflowStepCard | — | in-pr4 |
| `workflows.step-card.from-role-placeholder` | npub1…, hex pubkey, or role | WorkflowStepCard | — | in-pr4 |
| `workflows.step-card.message` | Message | WorkflowStepCard | — | in-pr4 |
| `workflows.step-card.message-text` | Message text | WorkflowStepCard | — | in-pr4 |
| `workflows.step-card.message-text-placeholder` | e.g. Deployment started by {{trigger.author}} | WorkflowStepCard | — | in-pr4 |
| `workflows.step-card.method` | HTTP method | WorkflowStepCard | — | in-pr4 |
| `workflows.step-card.name` | Name (optional) | WorkflowStepCard | — | in-pr4 |
| `workflows.step-card.name-placeholder` | e.g. Notify deployment channel | WorkflowStepCard | — | in-pr4 |
| `workflows.step-card.reply-in-thread` | Reply to triggering message in thread | WorkflowStepCard | — | in-pr4 |
| `workflows.step-card.request-body` | Request body (optional) | WorkflowStepCard | — | in-pr4 |
| `workflows.step-card.run-controls` | Run controls | WorkflowStepCard | — | in-pr4 |
| `workflows.step-card.run-controls-conditional` | Conditional | WorkflowStepCard | — | in-pr4 |
| `workflows.step-card.run-controls-conditional-timeout` | Conditional · {{timeout}} | WorkflowStepCard | — | in-pr4 |
| `workflows.step-card.run-controls-default` | Default | WorkflowStepCard | — | in-pr4 |
| `workflows.step-card.step-id` | Step ID | WorkflowStepCard | — | in-pr4 |
| `workflows.step-card.step-id-hint` | Used in configuration and run history. | WorkflowStepCard | — | in-pr4 |
| `workflows.step-card.timeout` | Timeout | WorkflowStepCard | — | in-pr4 |
| `workflows.step-card.timeout-hours-placeholder` | e.g. 24h | WorkflowStepCard | — | in-pr4 |
| `workflows.step-card.timeout-optional` | Timeout (optional) | WorkflowStepCard | — | in-pr4 |
| `workflows.step-card.to-pubkey` | To (pubkey) | WorkflowStepCard | — | in-pr4 |
| `workflows.step-card.to-pubkey-placeholder` | e.g. {{trigger.author}}, npub1…, or hex pubkey | WorkflowStepCard | — | in-pr4 |
| `workflows.step-card.topic` | Topic | WorkflowStepCard | — | in-pr4 |
| `workflows.step-card.topic-placeholder` | New channel topic | WorkflowStepCard | — | in-pr4 |
| `workflows.step-card.url-https-error` | URL must start with https:// | WorkflowStepCard | — | in-pr4 |
| `workflows.step-card.webhook-channel-warning` | This step will fail for webhook-triggered runs until a channel override is set. | WorkflowStepCard | — | in-pr4 |
| `workflows.text-condition.advanced-hint` | Use an evalexpr expression with | WorkflowMessageTextConditionEditor | — | in-pr4 |
| `workflows.text-condition.any` | Any | WorkflowMessageTextConditionEditor | — | in-pr4 |
| `workflows.text-condition.keep-advanced` | Keep advanced expression | WorkflowMessageTextConditionEditor | — | in-pr4 |
| `workflows.text-condition.replace-confirm` | Replace expression | WorkflowMessageTextConditionEditor | — | in-pr4 |
| `workflows.text-condition.replace-description` | Choosing a basic filter will replace the entire advanced expression. This cannot be undone. | WorkflowMessageTextConditionEditor | — | in-pr4 |
| `workflows.text-condition.replace-title` | Replace advanced expression? | WorkflowMessageTextConditionEditor | — | in-pr4 |
| `workflows.text-condition.replace-warning` | An advanced expression is active. Choosing a basic filter will replace it. | WorkflowMessageTextConditionEditor | — | in-pr4 |
| `workflows.text-condition.value-aria` | Text to match | WorkflowMessageTextConditionEditor | — | in-pr4 |
| `workflows.text-condition.value-placeholder` | e.g. deploy | WorkflowMessageTextConditionEditor | — | in-pr4 |
| `workflows.trigger-conditions.advanced-hint` | Use an evalexpr expression. Existing custom expressions stay unchanged. | WorkflowTriggerConditions | — | in-pr4 |
| `workflows.trigger-conditions.choose-emoji-aria` | Choose trigger emoji | WorkflowTriggerConditions | — | in-pr4 |
| `workflows.trigger-conditions.clear-emoji-aria` | Clear trigger emoji | WorkflowTriggerConditions | — | in-pr4 |
| `workflows.trigger-conditions.clear-filter` | Clear filter | WorkflowTriggerConditions | — | in-pr4 |
| `workflows.trigger-conditions.excluded-author` | Excluded author: {{label}} | WorkflowTriggerConditions | — | in-pr4 |
| `workflows.trigger-conditions.excluded-emoji` | Excluded reaction emoji: {{emoji}} | WorkflowTriggerConditions | — | in-pr4 |
| `workflows.trigger-conditions.replace-basic` | Replace with basic filters | WorkflowTriggerConditions | — | in-pr4 |
| `workflows.trigger-conditions.replace-warning` | An advanced expression is active. Replacing it with basic filters cannot be undone. | WorkflowTriggerConditions | — | in-pr4 |
| `workflows.trigger-conditions.selected-author` | Selected author: {{label}} | WorkflowTriggerConditions | — | in-pr4 |
| `workflows.trigger-conditions.selected-emoji` | Selected reaction emoji: {{emoji}} | WorkflowTriggerConditions | — | in-pr4 |
| `workflows.unavailable.description` | This workflow could not be loaded. It may no longer be available to you. | WorkflowUnavailableDialog | — | in-pr4 |
| `workflows.unavailable.loading` | Loading workflow… | WorkflowUnavailableDialog | — | in-pr4 |
| `workflows.unavailable.loading-aria` | Loading workflow | WorkflowUnavailableDialog | — | in-pr4 |
| `workflows.unavailable.loading-description` | Resolving workflow details. | WorkflowUnavailableDialog | — | in-pr4 |
| `workflows.unavailable.retry` | Retry | WorkflowUnavailableDialog | — | in-pr4 |
| `workflows.unavailable.title` | Workflow unavailable | WorkflowUnavailableDialog | — | in-pr4 |
| `workflows.view.create-aria` | Create Workflow | WorkflowsView | — | in-pr4 |
| `workflows.view.description` | Automations that keep your community moving. | WorkflowsView | — | in-pr4 |
| `workflows.view.load-failed` | Failed to load workflows | WorkflowsView | — | in-pr4 |
| `workflows.view.refresh-aria` | Refresh workflows | WorkflowsView | — | in-pr4 |
| `workflows.view.retry` | Retry | WorkflowsView | — | in-pr4 |
| `workflows.view.status-toggle-failed` | Couldn’t change workflow status | WorkflowsView | — | in-pr4 |
| `workflows.view.status-toggle-failed-description` | The workflow was not changed. Try again. | WorkflowsView | — | in-pr4 |
| `workflows.view.title` | Workflows | WorkflowsView | — | in-pr4 |
| `workflows.webhook.copy-secret` | Copy Secret | WorkflowWebhookSecretDialog | — | in-pr4 |
| `workflows.webhook.copy-url` | Copy URL | WorkflowWebhookSecretDialog | — | in-pr4 |
