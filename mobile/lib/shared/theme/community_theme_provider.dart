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

    _storage = CommunityThemeStorage(ref.read(savedPrefsProvider));
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
    final fallback = _storage.hasMigrated(pubkey)
        ? defaultCommunityTheme
        : _storage.legacyPreference();
    final initial = cached ?? fallback;

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
      );
      _manager = manager;
      Future.microtask(() async {
        final result = await manager.initialize(initial);
        if (_manager != manager) return;
        if (result.status == CommunityThemeRemoteStatus.valid ||
            result.status == CommunityThemeRemoteStatus.absent) {
          if (result.status == CommunityThemeRemoteStatus.absent) {
            await _storage.write(pubkey, config.baseUrl, initial);
          }
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
    unawaited(_storage.write(pubkey, relayUrl, preference));
    _manager?.publish(preference);
  }

  void _applyRemote(
    CommunityThemeSyncManager manager,
    RemoteCommunityTheme remote,
  ) {
    if (_manager != manager) return;
    state = remote.preference;
    final pubkey = _pubkey;
    final relayUrl = _relayUrl;
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

final communityThemeProvider =
    NotifierProvider<CommunityThemeNotifier, CommunityThemePreference>(
      CommunityThemeNotifier.new,
    );
