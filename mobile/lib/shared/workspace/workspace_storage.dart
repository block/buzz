import 'dart:convert';

import 'package:flutter_secure_storage/flutter_secure_storage.dart';

import 'workspace.dart';

class WorkspaceStorage {
  static const _keyWorkspaces = 'buzz_workspaces';
  static const _keyActiveId = 'buzz_active_workspace_id';

  // Legacy keys for migration.
  static const _legacyRelayUrl = 'buzz_relay_url';
  static const _legacyToken = 'buzz_token';
  static const _legacyPubkey = 'buzz_pubkey';
  static const _legacyNsec = 'buzz_nsec';

  final FlutterSecureStorage _secure;

  WorkspaceStorage({FlutterSecureStorage? secure})
    : _secure = secure ?? const FlutterSecureStorage();

  /// Load all workspaces. On first call, migrates legacy single-workspace
  /// credentials if present.
  Future<List<Workspace>> loadAll() async {
    final raw = await _secure.read(key: _keyWorkspaces);
    if (raw != null) {
      final list = jsonDecode(raw) as List<dynamic>;
      return list
          .map((e) => Workspace.fromJson(e as Map<String, dynamic>))
          .toList();
    }

    // Migration: check for legacy keys.
    final legacyUrl = await _secure.read(key: _legacyRelayUrl);
    final legacyToken = await _secure.read(key: _legacyToken);
    if (legacyUrl != null && legacyToken != null) {
      final legacyPubkey = await _secure.read(key: _legacyPubkey);
      final legacyNsec = await _secure.read(key: _legacyNsec);

      final name = Workspace.nameFromUrl(legacyUrl);
      final workspace = Workspace.create(
        name: name,
        relayUrl: legacyUrl,
        pubkey: legacyPubkey,
        nsec: legacyNsec,
      );

      await _saveList([workspace]);
      await saveActiveId(workspace.id);

      // Delete legacy keys.
      await _secure.delete(key: _legacyRelayUrl);
      await _secure.delete(key: _legacyToken);
      await _secure.delete(key: _legacyPubkey);
      await _secure.delete(key: _legacyNsec);

      return [workspace];
    }

    return [];
  }

  Future<void> save(Workspace workspace) async {
    final all = await loadAll();
    final index = all.indexWhere((w) => w.id == workspace.id);
    if (index >= 0) {
      all[index] = workspace;
    } else {
      all.add(workspace);
    }
    await _saveList(all);
  }

  Future<void> remove(String id) async {
    final all = await loadAll();
    all.removeWhere((w) => w.id == id);
    await _saveList(all);
  }

  Future<String?> loadActiveId() async {
    return _secure.read(key: _keyActiveId);
  }

  Future<void> saveActiveId(String id) async {
    await _secure.write(key: _keyActiveId, value: id);
  }

  Future<void> clearActiveId() async {
    await _secure.delete(key: _keyActiveId);
  }

  Future<void> _saveList(List<Workspace> workspaces) async {
    final json = jsonEncode(workspaces.map((w) => w.toJson()).toList());
    await _secure.write(key: _keyWorkspaces, value: json);
  }
}
