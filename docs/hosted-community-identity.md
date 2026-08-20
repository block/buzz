# Hosted communities and Buzz identity

Builderlab (hosted community) accounts are **email logins**. Buzz identities
are **Nostr keypairs** stored in the OS keyring on each device. Connecting
binds the current device key to that Builderlab account — it does **not**
generate a second identity for the email.

## Common confusion (#3880)

Symptom: after signing into Builderlab with a work email, the app asks to
"Finish connecting Buzz" / "Connect this device's Buzz identity," and there
is no "create a new private key" button on that screen.

That is intentional. The bind step always uses the key already on the device.

### Want a different key for this Builderlab email?

1. Back up the current nsec (Settings → Account → private key backup).
2. Sign out of the Buzz identity (Settings → Account → Sign out).
3. Import an nsec or create a fresh identity during onboarding.
4. Return to hosted community setup and connect again.

### Want the same Buzz identity on another machine?

Import the same nsec on that machine (do not generate a new key), then connect
the Builderlab account. Binding a different local key to an account that is
already linked shows the mismatch dialog ("This account uses a different Buzz
identity").

### Want a different Builderlab email with the same key?

Sign out of Builderlab (hosted community flow / settings), sign in with the
other email, then connect — the local Buzz key can be linked to multiple
Builderlab accounts over time, but each account stores one linked npub.
