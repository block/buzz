import 'dart:convert';

import 'package:flutter/foundation.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../relay/relay_provider.dart' show RelayConfig;
import '../theme/community_theme_preference.dart';
import '../theme/theme_provider.dart';

const _keyPrefix = 'buzz.channel-list.v1';
const _version = 1;

/// Snapshots beyond these bounds are not stored; the list loads from the relay.
const maxChannelListSnapshotChannels = 2000;
const maxChannelListSnapshotBytes = 2 * 1024 * 1024;

/// A disposable, relay-derived channel list for one relay and identity.
class ChannelListSnapshot {
  const ChannelListSnapshot({
    required this.channels,
    required this.hiddenDmIds,
  });

  /// Member channel JSON in the shape of `Channel.toJson`.
  final List<Map<String, dynamic>> channels;
  final Set<String> hiddenDmIds;
}

/// Permission to write one scope's snapshot, captured before a network fetch.
///
/// [ChannelListSnapshotStorage.remove] retires every token captured before
/// it, so a fetch in flight during removal cannot recreate the snapshot.
class ChannelListSnapshotToken {
  const ChannelListSnapshotToken._(this._key, this._epoch);

  final String _key;
  final int _epoch;
}

/// Stores the last authoritative channel list per relay and identity.
///
/// The data is a cache derived from relay events: unreadable, unsupported or
/// oversized data is discarded rather than migrated.
class ChannelListSnapshotStorage {
  ChannelListSnapshotStorage(this._prefs);

  final SharedPreferences _prefs;

  /// Retirement epochs shared by every storage over the same preferences.
  static final _epochsByPrefs = Expando<Map<String, int>>();
  static final _savedValuesByPrefs = Expando<Map<String, String>>();

  Map<String, String> get _savedValues => _savedValuesByPrefs[_prefs] ??= {};

  Map<String, int> get _epochs => _epochsByPrefs[_prefs] ??= {};

  /// The scope key. `wss://`/`ws://` origins fold to `https://`/`http://`
  /// like `RelayConfig.baseUrl`, so both onboarding paths share a snapshot.
  static String key(String pubkey, String relayUrl) {
    final relay = normalizeCommunityRelayUrl(
      RelayConfig(baseUrl: normalizeCommunityRelayUrl(relayUrl)).baseUrl,
    );
    return '$_keyPrefix:${pubkey.toLowerCase()}:${Uri.encodeComponent(relay)}';
  }

  /// Returns the scope's snapshot, or null when absent or unusable.
  ChannelListSnapshot? read(String pubkey, String relayUrl) {
    final key = ChannelListSnapshotStorage.key(pubkey, relayUrl);
    final raw = _prefs.getString(key);
    if (raw == null) return null;
    try {
      // UTF-16 length is at most the UTF-8 length, so this skips encoding
      // for the oversized case before measuring the exact bound.
      if (raw.length > maxChannelListSnapshotBytes ||
          utf8.encode(raw).length > maxChannelListSnapshotBytes) {
        throw const FormatException('size');
      }
      final json = jsonDecode(raw) as Map<String, dynamic>;
      if (json['v'] != _version) throw const FormatException('version');
      final channels = json['channels'] as List<dynamic>;
      if (channels.length > maxChannelListSnapshotChannels) {
        throw const FormatException('count');
      }
      return ChannelListSnapshot(
        channels: [
          for (final channel in channels) channel as Map<String, dynamic>,
        ],
        hiddenDmIds: {
          for (final id in json['hidden_dm_ids'] as List<dynamic>) id as String,
        },
      );
    } catch (_) {
      // Unusable cache: drop it without retiring in-flight writers.
      _savedValues.remove(key);
      _prefs.remove(key).ignore();
      return null;
    }
  }

  /// Captures write permission for the scope before its fetch starts.
  ChannelListSnapshotToken capture(String pubkey, String relayUrl) {
    final key = ChannelListSnapshotStorage.key(pubkey, relayUrl);
    return ChannelListSnapshotToken._(key, _epochs[key] ?? 0);
  }

  /// Replaces the scope's snapshot. Returns false when [token] was retired,
  /// the snapshot exceeds the size bounds, or storage rejected the write.
  Future<bool> write(
    ChannelListSnapshotToken token,
    ChannelListSnapshot snapshot,
  ) async {
    if ((_epochs[token._key] ?? 0) != token._epoch) return false;
    final raw = snapshot.channels.length > maxChannelListSnapshotChannels
        ? null
        : jsonEncode({
            'v': _version,
            'channels': snapshot.channels,
            'hidden_dm_ids': snapshot.hiddenDmIds.toList()..sort(),
          });
    if (raw == null || utf8.encode(raw).length > maxChannelListSnapshotBytes) {
      // An older, smaller snapshot no longer describes this list.
      _savedValues.remove(token._key);
      await _prefs.remove(token._key);
      return false;
    }
    if (_savedValues[token._key] == raw &&
        _prefs.getString(token._key) == raw) {
      return true;
    }
    // The token check and SharedPreferences' synchronous in-memory update run
    // without an intervening await, so a later remove always wins.
    // Failed platform writes still change SharedPreferences' memory cache.
    // Only skip values known to have been successfully persisted.
    _savedValues.remove(token._key);
    final saved = await _prefs.setString(token._key, raw);
    if (saved && (_epochs[token._key] ?? 0) == token._epoch) {
      _savedValues[token._key] = raw;
    }
    return saved;
  }

  /// Deletes the scope's snapshot and retires its outstanding tokens.
  ///
  /// Throws on storage failure. Community sign-out treats this as best-effort;
  /// orphan cleanup retries the remaining disk entry.
  Future<void> remove(String pubkey, String relayUrl) async {
    await _removeKey(ChannelListSnapshotStorage.key(pubkey, relayUrl));
  }

  /// Retires and deletes snapshots without a stored community. Failed deletes
  /// remain on disk for retry at the next startup or community mutation.
  Future<void> removeOrphans(Set<String> retainedKeys) async {
    final keys = {..._prefs.getKeys(), ..._epochs.keys};
    // Start every deletion before yielding: later community additions must not
    // have a new writer retired by an older cleanup's delayed next iteration.
    await Future.wait([
      for (final key in keys)
        if (key.startsWith('$_keyPrefix:') && !retainedKeys.contains(key))
          _removeKey(key).catchError((Object error) {
            debugPrint('[ChannelListSnapshotStorage] cleanup failed: $error');
          }),
    ]);
  }

  Future<void> _removeKey(String key) async {
    _savedValues.remove(key);
    _epochs[key] = (_epochs[key] ?? 0) + 1;
    if (!await _prefs.remove(key)) {
      throw StateError('Channel list snapshot could not be removed.');
    }
  }
}

/// The app's snapshot storage, or null where preferences were never provided
/// (isolated tests). Null means there is no snapshot to read, write or remove.
final channelListSnapshotStorageProvider =
    Provider<ChannelListSnapshotStorage?>((ref) {
      final SharedPreferences prefs;
      try {
        prefs = ref.watch(savedPrefsProvider);
      } catch (_) {
        // The unprovided override throws, wrapped by Riverpod.
        return null;
      }
      return ChannelListSnapshotStorage(prefs);
    });
