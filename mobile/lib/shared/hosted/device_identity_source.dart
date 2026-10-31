import 'dart:convert';

import 'package:flutter_secure_storage/flutter_secure_storage.dart';

import '../storage/secure_storage.dart';

import 'account_identity_store.dart';
import '../profile/user_profile.dart';
import '../relay/relay_provider.dart';

/// Public identity choices; never contains private signing material.
class DeviceIdentityChoice {
  const DeviceIdentityChoice(this.pubkey, this.label, {this.fromCloud = false});
  final String pubkey;
  final String label;
  final bool fromCloud;
}

/// Read-only inventory, including legacy records hidden by a newer community list.
/// Do not call CommunityStorage.loadAll here: it migrates and deletes records.
class DeviceIdentitySource {
  DeviceIdentitySource({FlutterSecureStorage? storage})
    : _storage = storage ?? communitySecureStorage;
  final FlutterSecureStorage _storage;

  Future<List<DeviceIdentityChoice>> choices() async => [
    for (final entry in (await _read()).entries)
      DeviceIdentityChoice(entry.key, entry.value.label),
  ];

  /// Read profile metadata only from communities already saved for this key.
  /// A failed/offline community does not prevent trying another saved source.
  Future<UserProfile?> loadProfile(
    String pubkey,
    Future<UserProfile?> Function(RelayConfig config) fetch,
  ) async {
    final seen = <String>{};
    for (final key in ['buzz_communities', 'buzz_workspaces']) {
      final raw = await _storage.read(key: key);
      if (raw == null) continue;
      for (final record in jsonDecode(raw) as List) {
        try {
          final value = record as Map<String, dynamic>;
          final nsec = value['nsec'] as String?;
          final relay = value['relayUrl'] as String?;
          if (nsec == null ||
              relay == null ||
              AccountIdentityStore.publicKeyForNsec(nsec) != pubkey ||
              (value['pubkey'] != null && value['pubkey'] != pubkey)) {
            continue;
          }
          final config = RelayConfig(baseUrl: relay, nsec: nsec);
          if (!seen.add(config.baseUrl)) continue;
          if (seen.length > 3) return null;
          final profile = await fetch(config);
          if (profile != null) return profile;
        } catch (_) {
          // Metadata is optional; never alter keys or block identity selection.
        }
      }
    }
    return null;
  }

  /// Reread and validate the chosen key immediately before create-only adoption.
  Future<AccountIdentitySnapshot> adopt(
    String pubkey,
    AccountIdentityStore store, {
    required void Function() checkCurrent,
  }) async {
    final candidate = (await _read())[pubkey];
    if (candidate == null) throw AccountIdentityFailure.absent;
    checkCurrent();
    return store.create(candidate.nsec, checkCurrent: checkCurrent);
  }

  /// Supplies a freshly validated saved identity only to an admission service.
  Future<T> withIdentity<T>(
    String pubkey,
    Future<T> Function(String nsec) operation,
  ) async {
    final candidate = (await _read())[pubkey];
    if (candidate == null) throw AccountIdentityFailure.absent;
    return operation(candidate.nsec);
  }

  Future<Map<String, ({String nsec, String label})>> _read() async {
    try {
      final result = <String, ({String nsec, String label})>{};
      final labels = <String, Set<String>>{};
      void add(dynamic nsec, dynamic advertised, String label) {
        if (nsec == null) {
          // Token-only legacy communities legitimately have no private key.
          return;
        }
        if (nsec is! String) throw AccountIdentityFailure.unreadable;
        final pubkey = AccountIdentityStore.publicKeyForNsec(nsec);
        if (advertised != null && advertised != pubkey) {
          throw AccountIdentityFailure.unreadable;
        }
        final names = labels.putIfAbsent(pubkey, () => <String>{})..add(label);
        result[pubkey] = (nsec: nsec, label: names.join(', '));
      }

      for (final key in ['buzz_communities', 'buzz_workspaces']) {
        final raw = await _storage.read(key: key);
        if (raw == null) continue;
        final records = jsonDecode(raw) as List;
        for (final record in records) {
          final value = record as Map<String, dynamic>;
          add(
            value['nsec'],
            value['pubkey'],
            (value['name'] as String?)?.trim().isNotEmpty == true
                ? (value['name'] as String).trim()
                : 'Saved community',
          );
        }
      }
      add(
        await _storage.read(key: 'buzz_nsec'),
        await _storage.read(key: 'buzz_pubkey'),
        'Previous device identity',
      );
      return result;
    } catch (_) {
      throw AccountIdentityFailure.unreadable;
    }
  }
}
