import 'dart:convert';

import 'package:flutter/foundation.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../relay/relay_provider.dart' show RelayConfig;
import '../theme/community_theme_preference.dart';
import '../theme/theme_provider.dart';
import 'profile_event_parser.dart';
import 'user_profile.dart';

const _keyPrefix = 'buzz.profiles.v1';
const _version = 1;

/// Profiles beyond these bounds are not stored; they load from the relay.
const maxProfileSnapshotProfiles = 2000;
const maxProfileSnapshotBytes = 2 * 1024 * 1024;

/// A single stored profile may not exceed this. Inline `data:` avatars can be
/// large; such profiles are skipped rather than crowding out everyone else.
const maxProfileSnapshotEntryBytes = 16 * 1024;

/// Permission to write one scope's snapshot, captured when the cache starts.
///
/// [ProfileSnapshotStorage.remove] retires every token captured before it, so
/// a pending write during sign-out cannot recreate the snapshot.
class ProfileSnapshotToken {
  const ProfileSnapshotToken._(this._key, this._epoch);

  final String _key;
  final int _epoch;
}

/// Stores the last verified profiles seen per relay and identity, so names and
/// avatars render on launch before the relay answers.
///
/// The data is a cache derived from relay kind:0 events: unreadable,
/// unsupported or oversized data is discarded rather than migrated. Restored
/// profiles are revalidated against the relay on first use each session.
class ProfileSnapshotStorage {
  ProfileSnapshotStorage(this._prefs);

  final SharedPreferences _prefs;

  /// Retirement epochs shared by every storage over the same preferences.
  static final _epochsByPrefs = Expando<Map<String, int>>();

  Map<String, int> get _epochs => _epochsByPrefs[_prefs] ??= {};

  /// The scope key. `wss://`/`ws://` origins fold to `https://`/`http://`
  /// like `RelayConfig.baseUrl`, so both onboarding paths share a snapshot.
  static String key(String pubkey, String relayUrl) {
    final relay = normalizeCommunityRelayUrl(
      RelayConfig(baseUrl: normalizeCommunityRelayUrl(relayUrl)).baseUrl,
    );
    return '$_keyPrefix:${pubkey.toLowerCase()}:${Uri.encodeComponent(relay)}';
  }

  /// Returns the scope's profiles, or an empty list when absent or unusable.
  List<ParsedProfileEvent> read(String pubkey, String relayUrl) {
    final key = ProfileSnapshotStorage.key(pubkey, relayUrl);
    final raw = _prefs.getString(key);
    if (raw == null) return const [];
    try {
      if (raw.length > maxProfileSnapshotBytes ||
          utf8.encode(raw).length > maxProfileSnapshotBytes) {
        throw const FormatException('size');
      }
      final json = jsonDecode(raw) as Map<String, dynamic>;
      if (json['v'] != _version) throw const FormatException('version');
      final profiles = json['profiles'] as List<dynamic>;
      if (profiles.length > maxProfileSnapshotProfiles) {
        throw const FormatException('count');
      }
      return [
        for (final entry in profiles) _decode(entry as Map<String, dynamic>),
      ];
    } catch (_) {
      // Unusable cache: drop it without retiring the live writer.
      _prefs.remove(key).ignore();
      return const [];
    }
  }

  /// Captures write permission for the scope.
  ProfileSnapshotToken capture(String pubkey, String relayUrl) {
    final key = ProfileSnapshotStorage.key(pubkey, relayUrl);
    return ProfileSnapshotToken._(key, _epochs[key] ?? 0);
  }

  /// Replaces the scope's snapshot with the newest [profiles] that fit the
  /// bounds. Returns false when [token] was retired or storage rejected it.
  Future<bool> write(
    ProfileSnapshotToken token,
    Iterable<ParsedProfileEvent> profiles,
  ) async {
    if ((_epochs[token._key] ?? 0) != token._epoch) return false;
    final newestFirst = profiles.toList()
      ..sort((a, b) => b.createdAt.compareTo(a.createdAt));
    final entries = <String>[];
    // Two bytes per entry cover the list's brackets and separators.
    var bytes = 32;
    for (final profile in newestFirst) {
      if (entries.length == maxProfileSnapshotProfiles) break;
      final entry = jsonEncode(_encode(profile));
      final entryBytes = utf8.encode(entry).length;
      if (entryBytes > maxProfileSnapshotEntryBytes) continue;
      if (bytes + entryBytes + 1 > maxProfileSnapshotBytes) break;
      bytes += entryBytes + 1;
      entries.add(entry);
    }
    final raw = '{"v":$_version,"profiles":[${entries.join(',')}]}';
    if (_prefs.getString(token._key) == raw) return true;
    // The token check and SharedPreferences' synchronous in-memory update run
    // without an intervening await, so a later remove always wins.
    return _prefs.setString(token._key, raw);
  }

  /// Deletes the scope's snapshot and retires its outstanding tokens.
  Future<void> remove(String pubkey, String relayUrl) =>
      _removeKey(ProfileSnapshotStorage.key(pubkey, relayUrl));

  /// Retires and deletes snapshots without a stored community.
  Future<void> removeOrphans(Set<String> retainedKeys) async {
    final keys = {..._prefs.getKeys(), ..._epochs.keys};
    await Future.wait([
      for (final key in keys)
        if (key.startsWith('$_keyPrefix:') && !retainedKeys.contains(key))
          _removeKey(key).catchError((Object error) {
            debugPrint('[ProfileSnapshotStorage] cleanup failed: $error');
          }),
    ]);
  }

  Future<void> _removeKey(String key) async {
    _epochs[key] = (_epochs[key] ?? 0) + 1;
    if (!await _prefs.remove(key)) {
      throw StateError('Profile snapshot could not be removed.');
    }
  }

  static Map<String, Object?> _encode(ParsedProfileEvent parsed) {
    final profile = parsed.profile;
    return {
      'pubkey': profile.pubkey,
      'event_id': parsed.eventId,
      'created_at': parsed.createdAt,
      'display_name': ?profile.displayName,
      'avatar_url': ?profile.avatarUrl,
      'about': ?profile.about,
      'nip05': ?profile.nip05Handle,
      'owner': ?profile.ownerPubkey,
    };
  }

  static ParsedProfileEvent _decode(Map<String, dynamic> json) => (
    profile: UserProfile(
      pubkey: (json['pubkey'] as String).toLowerCase(),
      displayName: json['display_name'] as String?,
      avatarUrl: json['avatar_url'] as String?,
      about: json['about'] as String?,
      nip05Handle: json['nip05'] as String?,
      ownerPubkey: json['owner'] as String?,
    ),
    createdAt: json['created_at'] as int,
    eventId: json['event_id'] as String,
  );
}

/// The app's profile snapshot storage, or null where preferences were never
/// provided (isolated tests). Null means profiles live only in memory.
final profileSnapshotStorageProvider = Provider<ProfileSnapshotStorage?>((ref) {
  final SharedPreferences prefs;
  try {
    prefs = ref.watch(savedPrefsProvider);
  } catch (_) {
    // The unprovided override throws, wrapped by Riverpod.
    return null;
  }
  return ProfileSnapshotStorage(prefs);
});
