import 'dart:async';

import 'package:flutter/material.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;

import '../crypto/nip44.dart';
import '../relay/relay.dart';
import 'accent_colors.dart';
import 'community_theme_preference.dart';
import 'community_theme_sync.dart';
import 'theme_provider.dart';

class CommunityThemeNotifier extends Notifier<CommunityThemePreference> {
  CommunityThemeSyncManager? _manager;
  late CommunityThemeStorage _storage;
  String? _pubkey;
  String? _relayUrl;

  @override
  CommunityThemePreference build() {
    _manager?.dispose();
    _manager = null;

    _storage = ref.watch(communityThemeStorageProvider);
    final config = ref.watch(relayConfigProvider);
    final session = ref.watch(relaySessionProvider);
    final pubkey = pubkeyFromNsec(config.nsec);
    _pubkey = pubkey;
    _relayUrl = config.baseUrl;

    if (pubkey == null || config.nsec == null) {
      final legacy = _storage.legacyPreference();
      unawaited(_storage.writeLegacy(legacy));
      return legacy;
    }

    final cached = _storage.read(pubkey, config.baseUrl);
    final dirty = _storage.readOutbox(pubkey, config.baseUrl);
    final fallback = _storage.hasMigrated(pubkey)
        ? defaultCommunityTheme
        : _storage.legacyPreference();
    final initial = dirty ?? cached ?? fallback;

    if (session.status == SessionStatus.connected) {
      late final CommunityThemeSyncManager manager;
      manager = CommunityThemeSyncManager(
        pubkey: pubkey,
        relaySession: ref.read(relaySessionProvider.notifier),
        signedEventRelay: SignedEventRelay(
          session: ref.read(relaySessionProvider.notifier),
          nsec: config.nsec!,
        ),
        crypto: _crypto(config.nsec!, pubkey),
        onRemote: (remote) => _applyRemote(manager, remote),
        onPublished: (preference) =>
            unawaited(_storage.clearOutbox(pubkey, config.baseUrl, preference)),
      );
      _manager = manager;
      if (dirty != null) manager.publish(dirty);
      Future.microtask(() async {
        final result = await manager.initialize();
        if (_manager != manager) return;
        if (result.status == CommunityThemeRemoteStatus.absent) {
          await _storage.write(pubkey, config.baseUrl, initial);
          await _storage.writeOutbox(pubkey, config.baseUrl, initial);
          if (_manager == manager) manager.publish(initial);
        }
        if (result.status == CommunityThemeRemoteStatus.valid ||
            result.status == CommunityThemeRemoteStatus.absent) {
          await _storage.markMigrated(pubkey);
        }
      });
      ref.onDispose(manager.dispose);
    }
    return initial;
  }

  void setMode(ThemeMode mode) {
    var theme = state.theme;
    if (mode == ThemeMode.system) {
      theme = schemeForAppearanceMode(theme, mode) ?? theme;
    } else {
      final effective = effectiveTheme(theme, mode);
      if (effective != null) theme = effective.name;
    }
    _save(
      CommunityThemePreference(
        theme: theme,
        accent: state.accent,
        followSystem: mode == ThemeMode.system,
      ),
    );
  }

  void setTheme(String? theme) {
    _save(
      CommunityThemePreference(
        theme: theme ?? defaultSchemeName,
        accent: state.accent,
        followSystem: state.followSystem,
      ),
    );
  }

  void setAccent(int index) {
    if (index < 0 || index >= accentColors.length) return;
    _save(
      CommunityThemePreference(
        theme: state.theme,
        accent: accentColors[index].wireValue,
        followSystem: state.followSystem,
      ),
    );
  }

  void _save(CommunityThemePreference preference) {
    if (preference == state) return;
    state = preference;
    final pubkey = _pubkey;
    final relayUrl = _relayUrl;
    if (pubkey == null || relayUrl == null) {
      unawaited(_storage.writeLegacy(preference));
      return;
    }
    _manager?.stage(preference);
    unawaited(_persistAndPublish(pubkey, relayUrl, preference));
  }

  Future<void> _persistAndPublish(
    String pubkey,
    String relayUrl,
    CommunityThemePreference preference,
  ) async {
    if (!await _storage.write(pubkey, relayUrl, preference)) return;
    if (!await _storage.writeOutbox(pubkey, relayUrl, preference)) return;
    if (_pubkey == pubkey && _relayUrl == relayUrl) {
      _manager?.publishStaged(preference);
    }
  }

  void _applyRemote(
    CommunityThemeSyncManager manager,
    RemoteCommunityTheme remote,
  ) {
    if (_manager != manager) return;
    final pubkey = _pubkey;
    final relayUrl = _relayUrl;
    if (pubkey != null &&
        relayUrl != null &&
        _storage.readOutbox(pubkey, relayUrl) != null) {
      final dirty = _storage.readOutbox(pubkey, relayUrl)!;
      manager.publish(dirty);
      return;
    }
    state = remote.preference;
    if (pubkey != null && relayUrl != null) {
      unawaited(_storage.write(pubkey, relayUrl, remote.preference));
    }
  }
}

CommunityThemeCrypto _crypto(String nsec, String pubkey) {
  final privateHex = nostr.Nip19.decode(payload: nsec).data;
  final key = getConversationKey(privateHex, pubkey);
  return CommunityThemeCrypto(
    encrypt: (plaintext) => nip44Encrypt(key, plaintext),
    decrypt: (ciphertext) => nip44Decrypt(key, ciphertext),
  );
}

final communityThemeStorageProvider = Provider<CommunityThemeStorage>(
  (ref) => CommunityThemeStorage(ref.watch(savedPrefsProvider)),
);

final communityThemeProvider =
    NotifierProvider<CommunityThemeNotifier, CommunityThemePreference>(
      CommunityThemeNotifier.new,
    );
