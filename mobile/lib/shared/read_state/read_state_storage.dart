import 'dart:convert';
import 'dart:math';

import 'package:flutter/foundation.dart';
import 'package:shared_preferences/shared_preferences.dart';
import 'package:uuid/uuid.dart';

import '../theme/community_theme_preference.dart';
import 'read_state_format.dart';
import 'read_state_time.dart';

const _contextsKeyPrefix = 'buzz.channel-read-state.v2';
const _publishableKeyPrefix = 'buzz.channel-read-state.publishable.v1';
const _sourceCreatedAtKeyPrefix =
    'buzz.channel-read-state.source-created-at.v1';
const _clientIdKeyPrefix = 'buzz.nip-rs.client-id';
const _slotIdKeyPrefix = 'buzz.nip-rs.slot-id';
const _uuid = Uuid();

/// Read marks belong to one community, so every key is scoped by relay as
/// well as identity. One identity can be paired with several communities.
String _scope(String pubkey, String relayUrl) =>
    '$pubkey:${Uri.encodeComponent(normalizeCommunityRelayUrl(relayUrl))}';

String localReadStateKey(String pubkey, String relayUrl) =>
    '$_contextsKeyPrefix:${_scope(pubkey, relayUrl)}';

String localPublishableContextKey(String pubkey, String relayUrl) =>
    '$_publishableKeyPrefix:${_scope(pubkey, relayUrl)}';

String localSourceCreatedAtKey(String pubkey, String relayUrl) =>
    '$_sourceCreatedAtKeyPrefix:${_scope(pubkey, relayUrl)}';

String clientIdKey(String pubkey, String relayUrl) =>
    '$_clientIdKeyPrefix:${_scope(pubkey, relayUrl)}';

String slotIdKey(String pubkey, String relayUrl) =>
    '$_slotIdKeyPrefix:${_scope(pubkey, relayUrl)}';

/// The earlier keys, shared by every community of the identity.
String _legacyKey(String prefix, String pubkey) => '$prefix:$pubkey';

class StoredReadState {
  final Map<String, int> contexts;
  final Set<String> publishableContextIds;
  final Map<String, int> sourceCreatedAt;

  StoredReadState({
    required Map<String, int> contexts,
    required Set<String> publishableContextIds,
    required Map<String, int> sourceCreatedAt,
  }) : contexts = Map.unmodifiable(contexts),
       publishableContextIds = Set.unmodifiable(publishableContextIds),
       sourceCreatedAt = Map.unmodifiable(sourceCreatedAt);
}

class ReadStateStorage {
  final SharedPreferences _prefs;

  ReadStateStorage(this._prefs);

  String getOrCreateClientId(String pubkey, String relayUrl) => _getOrCreateId(
    clientIdKey(pubkey, relayUrl),
    _legacyKey(_clientIdKeyPrefix, pubkey),
    isValid: _isValidClientId,
    generate: _uuid.v4,
  );

  String getOrCreateSlotId(String pubkey, String relayUrl) => _getOrCreateId(
    slotIdKey(pubkey, relayUrl),
    _legacyKey(_slotIdKeyPrefix, pubkey),
    isValid: _isValidSlotId,
    generate: generateReadStateSlotId,
  );

  /// Each community seeds its id from the identity-wide one when it has none,
  /// so it keeps publishing to the slot that already exists on its relay. The
  /// shared id stays for the other communities to seed from.
  String _getOrCreateId(
    String key,
    String legacyKey, {
    required bool Function(String?) isValid,
    required String Function() generate,
  }) {
    final stored = _prefs.getString(key);
    if (isValid(stored)) return stored!;

    final legacy = _prefs.getString(legacyKey);
    final id = isValid(legacy) ? legacy! : generate();
    _prefs.setString(key, id);
    return id;
  }

  void writeSlotId(String pubkey, String relayUrl, String slotId) {
    _prefs.setString(slotIdKey(pubkey, relayUrl), slotId);
  }

  StoredReadState read(String pubkey, String relayUrl) {
    // The shared marks may be another community's, and the relay keeps the
    // published ones, so drop them rather than let one community claim them.
    for (final prefix in const [
      _contextsKeyPrefix,
      _publishableKeyPrefix,
      _sourceCreatedAtKeyPrefix,
    ]) {
      _prefs.remove(_legacyKey(prefix, pubkey));
    }
    return StoredReadState(
      contexts: _readContexts(pubkey, relayUrl),
      publishableContextIds: _readPublishableContextIds(pubkey, relayUrl),
      sourceCreatedAt: _readSourceCreatedAt(pubkey, relayUrl),
    );
  }

  void write(
    String pubkey,
    String relayUrl,
    Map<String, int> contexts,
    Set<String> publishableContextIds,
    Map<String, int> sourceCreatedAt,
  ) {
    final state = <String, String>{};
    for (final entry in contexts.entries) {
      state[entry.key] = unixSecondsToDateTime(entry.value).toIso8601String();
    }

    _prefs.setString(localReadStateKey(pubkey, relayUrl), jsonEncode(state));
    _prefs.setString(
      localPublishableContextKey(pubkey, relayUrl),
      jsonEncode(publishableContextIds.toList()),
    );
    _prefs.setString(
      localSourceCreatedAtKey(pubkey, relayUrl),
      jsonEncode(sourceCreatedAt.map((k, v) => MapEntry(k, v.toString()))),
    );
  }

  Map<String, int> _readContexts(String pubkey, String relayUrl) {
    final raw = _prefs.getString(localReadStateKey(pubkey, relayUrl));
    if (raw == null || raw.isEmpty) {
      return {};
    }

    final Object? parsed;
    try {
      parsed = jsonDecode(raw);
    } catch (e) {
      debugPrint('[ReadStateManager] storage: contexts JSON corrupt: $e');
      return {};
    }

    final record = asStringObjectMap(parsed);
    if (record == null) {
      return {};
    }

    final contexts = <String, int>{};
    for (final entry in record.entries) {
      final timestamp = isoToUnixSeconds(entry.value);
      if (timestamp == null) continue;

      final current = contexts[entry.key] ?? 0;
      if (timestamp > current) {
        contexts[entry.key] = timestamp;
      }
    }
    return contexts;
  }

  Set<String> _readPublishableContextIds(String pubkey, String relayUrl) {
    final raw = _prefs.getString(localPublishableContextKey(pubkey, relayUrl));
    if (raw == null || raw.isEmpty) {
      return {};
    }

    final Object? parsed;
    try {
      parsed = jsonDecode(raw);
    } catch (e) {
      debugPrint(
        '[ReadStateManager] storage: publishableContextIds JSON corrupt: $e',
      );
      return {};
    }

    if (parsed is! List) {
      return {};
    }

    return {
      for (final value in parsed)
        if (value is String) value,
    };
  }

  Map<String, int> _readSourceCreatedAt(String pubkey, String relayUrl) {
    final raw = _prefs.getString(localSourceCreatedAtKey(pubkey, relayUrl));
    if (raw == null || raw.isEmpty) return {};

    final Object? parsed;
    try {
      parsed = jsonDecode(raw);
    } catch (e) {
      debugPrint(
        '[ReadStateManager] storage: sourceCreatedAt JSON corrupt: $e',
      );
      return {};
    }

    final record = asStringObjectMap(parsed);
    if (record == null) return {};

    final result = <String, int>{};
    for (final entry in record.entries) {
      final value = entry.value;
      if (value is int) {
        result[entry.key] = value;
      } else if (value is String) {
        final parsed = int.tryParse(value);
        if (parsed != null) result[entry.key] = parsed;
      }
    }
    return result;
  }
}

String generateReadStateSlotId() {
  final random = Random.secure();
  final bytes = List<int>.generate(16, (_) => random.nextInt(256));
  return bytes.map((byte) => byte.toRadixString(16).padLeft(2, '0')).join();
}

bool _isValidClientId(String? value) {
  return value != null && value.isNotEmpty && value.runes.length <= 64;
}

bool _isValidSlotId(String? value) {
  if (value == null || value.isEmpty || value.length > 64) {
    return false;
  }
  return isValidReadStateDTag('$readStateDTagPrefix$value');
}
