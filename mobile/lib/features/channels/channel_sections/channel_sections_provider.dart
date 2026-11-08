import 'package:flutter/widgets.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../../shared/relay/relay.dart';
import '../../../shared/theme/theme_provider.dart';
import '../../../shared/community/community_provider.dart';
import 'channel_sections_manager.dart';
import 'channel_sections_storage.dart';

class ChannelSectionsState {
  final bool isReady;
  final ChannelSectionStore store;

  /// Bumped on every change to force downstream rebuilds.
  final int version;

  const ChannelSectionsState({
    this.isReady = false,
    this.store = const ChannelSectionStore(),
    this.version = 0,
  });
}

class ChannelSectionsNotifier extends Notifier<ChannelSectionsState> {
  ChannelSectionsManager? _manager;
  bool _isInitialized = false;

  @override
  ChannelSectionsState build() {
    _manager?.dispose(flushPending: false);
    _manager = null;
    _isInitialized = false;

    final relayConfig = ref.watch(relayConfigProvider);
    final sessionState = ref.watch(relaySessionProvider);
    // Rebuild when the active community changes (pubkey may differ).
    ref.watch(activeCommunityProvider);

    final nsec = relayConfig.nsec?.trim();
    if (nsec == null || nsec.isEmpty) {
      return const ChannelSectionsState();
    }

    final pubkey = ref.watch(myPubkeyProvider);
    final crypto = ref.watch(channelSectionsCryptoProvider);
    if (pubkey == null || crypto == null) {
      return const ChannelSectionsState();
    }

    final prefs = ref.read(savedPrefsProvider);
    final signedRelay = SignedEventRelay(
      session: ref.read(relaySessionProvider.notifier),
      nsec: nsec,
    );

    late final ChannelSectionsManager manager;
    manager = ChannelSectionsManager(
      pubkey: pubkey,
      relayUrl: relayConfig.baseUrl,
      prefs: prefs,
      crypto: crypto,
      relaySession: ref.read(relaySessionProvider.notifier),
      signedEventRelay: signedRelay,
      remoteEnabled: sessionState.status == SessionStatus.connected,
      onChanged: () => _emitManagerState(manager),
    );
    _manager = manager;

    // Resume re-read catches an EVENT a healthy socket never delivered.
    ref.listen(appLifecycleProvider, (_, next) {
      if (next == AppLifecycleState.resumed) manager.refreshFromRelay();
    });

    ref.onDispose(() {
      manager.dispose();
      if (_manager == manager) {
        _manager = null;
      }
    });

    Future.microtask(() async {
      await manager.initialize();
      if (_manager != manager) return;
      _isInitialized = true;
      _emitManagerState(manager);
    });

    return ChannelSectionsState(
      isReady: false,
      store: manager.store,
      version: 1,
    );
  }

  void createSection(String name) => _manager?.createSection(name);

  void renameSection(String sectionId, String newName) =>
      _manager?.renameSection(sectionId, newName);

  void deleteSection(String sectionId) => _manager?.deleteSection(sectionId);

  void moveSectionUp(String sectionId) => _manager?.moveSectionUp(sectionId);

  void moveSectionDown(String sectionId) =>
      _manager?.moveSectionDown(sectionId);

  void assignChannel(String channelId, String sectionId) =>
      _manager?.assignChannel(channelId, sectionId);

  void unassignChannel(String channelId) =>
      _manager?.unassignChannel(channelId);

  void _emitManagerState(ChannelSectionsManager manager) {
    if (_manager != manager) return;
    state = ChannelSectionsState(
      isReady: _isInitialized,
      store: manager.store,
      version: state.version + 1,
    );
  }
}

final channelSectionsProvider =
    NotifierProvider<ChannelSectionsNotifier, ChannelSectionsState>(
      ChannelSectionsNotifier.new,
    );

/// Identity-scoped crypto survives connection status changes. A community or
/// signing-key change replaces it through [relayConfigProvider].
final channelSectionsCryptoProvider = Provider<ChannelSectionsCrypto?>((ref) {
  final config = ref.watch(relayConfigProvider);
  final pubkey = ref.watch(myPubkeyProvider);
  final nsec = config.nsec?.trim();
  if (nsec == null || nsec.isEmpty || pubkey == null) return null;
  try {
    return ChannelSectionsCrypto(nsec, pubkey);
  } catch (_) {
    return null;
  }
});
