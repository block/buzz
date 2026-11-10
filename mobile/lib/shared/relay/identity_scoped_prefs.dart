import 'dart:async';

import 'package:shared_preferences/shared_preferences.dart';

/// Reads an identity-scoped preference, migrating values left under a
/// pre-canonicalization relay origin.
///
/// Identity-scoped keys embed the relay origin, and that origin used to be
/// whatever scheme the onboarding flow happened to persist — `wss://` for an
/// invite join, `https://` for device pairing. Now that [RelayConfig.baseUrl]
/// canonicalizes the scheme, an install that joined by invite would compute a
/// different key than the one its data was written under, leaving that data on
/// disk but unreachable.
///
/// The value under [canonicalKey] wins. Otherwise a value under [legacyKey] is
/// returned straight away and promoted onto the canonical key in the
/// background. The legacy entry is removed only once the copy reports success,
/// so a migration interrupted mid-flight leaves the original readable and the
/// next read simply retries.
///
/// Pass matching accessors for the stored type — `getString`/`setString`, or
/// `getStringList`/`setStringList`.
T? readMigratedPref<T>(
  SharedPreferences prefs, {
  required String canonicalKey,
  required String legacyKey,
  required T? Function(String key) read,
  required Future<bool> Function(String key, T value) write,
}) {
  final canonical = read(canonicalKey);
  if (canonical != null) return canonical;

  // Pairing-created communities already store an HTTP origin, so the two keys
  // coincide and there is nothing to migrate.
  if (canonicalKey == legacyKey) return null;

  final legacy = read(legacyKey);
  if (legacy == null) return null;

  unawaited(_promote(prefs, legacyKey, write(canonicalKey, legacy)));
  return legacy;
}

Future<void> _promote(
  SharedPreferences prefs,
  String legacyKey,
  Future<bool> copy,
) async {
  if (await copy) await prefs.remove(legacyKey);
}
