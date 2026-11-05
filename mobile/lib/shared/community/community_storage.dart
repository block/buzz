import 'dart:convert';

import 'package:crypto/crypto.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';

import '../storage/secure_storage.dart';
import '../storage/serial_queue.dart';

import 'community.dart';

class CommunityStorage {
  static const _keyCommunities = 'buzz_communities';
  static const _keyActiveId = 'buzz_active_community_id';

  // Legacy keys for migration.
  static const _legacyCommunities = 'buzz_workspaces';
  static const _legacyActiveId = 'buzz_active_workspace_id';
  static const _legacyRelayUrl = 'buzz_relay_url';
  static const _legacyToken = 'buzz_token';
  static const _legacyPubkey = 'buzz_pubkey';
  static const _legacyNsec = 'buzz_nsec';

  final FlutterSecureStorage _secure;

  CommunityStorage({FlutterSecureStorage? secure})
    : _secure = secure ?? communitySecureStorage;

  /// Load all communities. On first call, migrates legacy single-community
  /// credentials if present.
  Future<List<Community>> loadAll() async {
    final raw = await _secure.read(key: _keyCommunities);
    if (raw != null) return _decodeList(raw);

    final legacyCommunities = await _secure.read(key: _legacyCommunities);
    if (legacyCommunities != null) {
      final communities = _decodeList(legacyCommunities);
      await _saveList(communities);
      final legacyActiveId = await _secure.read(key: _legacyActiveId);
      if (legacyActiveId != null) await saveActiveId(legacyActiveId);
      await _secure.delete(key: _legacyCommunities);
      await _secure.delete(key: _legacyActiveId);
      return communities;
    }

    // Migration: check for legacy single-community keys.
    final legacyUrl = await _secure.read(key: _legacyRelayUrl);
    final legacyToken = await _secure.read(key: _legacyToken);
    if (legacyUrl != null && legacyToken != null) {
      final legacyPubkey = await _secure.read(key: _legacyPubkey);
      final legacyNsec = await _secure.read(key: _legacyNsec);

      final name = Community.nameFromUrl(legacyUrl);
      final community = Community.create(
        name: name,
        relayUrl: legacyUrl,
        pubkey: legacyPubkey,
        nsec: legacyNsec,
        sensitiveActionPolicy: SensitiveActionPolicy.disabledByUser,
      );

      await _saveList([community]);
      await saveActiveId(community.id);

      // Delete legacy keys.
      await _secure.delete(key: _legacyRelayUrl);
      await _secure.delete(key: _legacyToken);
      await _secure.delete(key: _legacyPubkey);
      await _secure.delete(key: _legacyNsec);

      return [community];
    }

    return [];
  }

  static final _pendingInvites = SerialQueue();
  static const _pendingInvitePrefix = 'buzz_pending_invite_v1_';

  static String _pendingInviteKey(String relayUrl, String code) {
    final uri = Uri.parse(relayUrl);
    final origin = uri
        .replace(
          scheme: ['https', 'wss'].contains(uri.scheme) ? 'https' : 'http',
        )
        .origin;
    return '$_pendingInvitePrefix${sha256.convert(utf8.encode(jsonEncode([origin, code])))}';
  }

  /// Retain the claim identity before sending a request with uncertain outcomes.
  /// A failed read/write must never generate a replacement for an unknown key.
  Future<String> pendingInviteIdentity(
    String relayUrl,
    String code,
    String Function() generate,
  ) => _pendingInvites.run(() async {
    final key = _pendingInviteKey(relayUrl, code);
    final saved = await _secure.read(key: key);
    if (saved != null) return saved;
    final nsec = generate();
    await _secure.write(key: key, value: nsec);
    if (await _secure.read(key: key) != nsec) {
      throw StateError('Could not retain the pending invite identity');
    }
    return nsec;
  });

  /// Retire the journal only after the destination identity is durably saved.
  Future<void> completePendingInvite(
    String relayUrl,
    String code,
    Community community,
  ) => _pendingInvites.run(() async {
    final saved = await loadAll();
    if (!saved.any(
      (item) => item.id == community.id && item.nsec == community.nsec,
    )) {
      return;
    }
    final key = _pendingInviteKey(relayUrl, code);
    if (await _secure.read(key: key) == community.nsec) {
      await _secure.delete(key: key);
    }
  });

  Future<void> save(Community community) async {
    final all = await loadAll();
    final index = all.indexWhere((w) => w.id == community.id);
    if (index >= 0) {
      all[index] = community;
    } else {
      all.add(community);
    }
    await _saveList(all);
  }

  /// Replaces the complete stored community list in one secure-storage write.
  Future<void> saveAll(List<Community> communities) => _saveList(communities);

  Future<void> remove(String id) => _pendingInvites.run(() async {
    final all = await loadAll();
    final removedKeys = all
        .where((item) => item.id == id)
        .map((item) => item.nsec)
        .whereType<String>()
        .toSet();
    if (removedKeys.isNotEmpty) {
      // Retire every journal copy before removing the saved community. If a
      // deletion fails, keep that community so sign-out can safely retry.
      final records = await _secure.readAll();
      for (final entry in records.entries) {
        if (entry.key.startsWith(_pendingInvitePrefix) &&
            removedKeys.contains(entry.value)) {
          await _secure.delete(key: entry.key);
          if (await _secure.read(key: entry.key) != null) {
            throw StateError('Could not remove the pending invite identity');
          }
        }
      }
    }
    all.removeWhere((w) => w.id == id);
    await _saveList(all);
  });

  Future<String?> loadActiveId() async {
    return _secure.read(key: _keyActiveId);
  }

  Future<void> saveActiveId(String id) async {
    await _secure.write(key: _keyActiveId, value: id);
  }

  Future<void> clearActiveId() async {
    await _secure.delete(key: _keyActiveId);
  }

  List<Community> _decodeList(String raw) {
    final list = jsonDecode(raw) as List<dynamic>;
    return list
        .map((entry) => Community.fromJson(entry as Map<String, dynamic>))
        .toList();
  }

  Future<void> _saveList(List<Community> communities) async {
    final json = jsonEncode(communities.map((item) => item.toJson()).toList());
    await _secure.write(key: _keyCommunities, value: json);
  }
}
