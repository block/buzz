# Android push enrollment requires hardware key attestation on a locked bootloader

The push gateway's enrollment endpoints are public and have no other authentication, so an installation proves it is the gateway's app on real hardware before it can enroll: App Attest on iOS, Android Key Attestation on Android. The gateway verifies Key Attestation offline, as it does App Attest, against Google's attestation roots and its published revocation list (the gateway's one periodic online fetch). It accepts an attestation only when the hardware reports a locked bootloader with a verified boot state of Verified or SelfSigned, because the app identity in the attestation (package name and signing certificate) is reported by the operating system, so on an unlocked bootloader it proves nothing. Android builds that run with a locked bootloader and their own signing key, such as GrapheneOS, pass. Phones without hardware-backed attestation get no push; there is no software fallback. Enrollment keeps its existing challenge quotas and adds a rate limit per attested key.

## Considered options

- **Play Integrity.** It would send Google a request for every enrollment, which works against Buzz's privacy goals, and it adds Google's server API and its quotas as a dependency.
- **Accept any boot state.** It would let more phones enroll, but a rooted phone could then claim to be any app, which leaves the endpoints open to the abuse attestation is meant to stop.
