import 'package:flutter/widgets.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../../shared/relay/relay.dart';
import '../../../shared/theme/theme_provider.dart';
import '../../../shared/community/community_provider.dart';
import 'channel_mutes_manager.dart';
import 'channel_mutes_storage.dart';

class ChannelMutesState {
  final bool isReady;
  final ChannelMuteStore store;

  /// Bumped on every change to force downstream rebuilds.
  final int version;

  const ChannelMutesState({
    this.isReady = false,
    this.store = const ChannelMuteStore(),
    this.version = 0,
  });
}

class ChannelMutesNotifier extends Notifier<ChannelMutesState> {
  ChannelMutesManager? _manager;
  bool _isInitialized = false;

  @override
  ChannelMutesState build() {
    _manager?.dispose(flushPending: false);
    _manager = null;
    _isInitialized = false;

    final relayConfig = ref.watch(relayConfigProvider);
    final sessionState = ref.watch(relaySessionProvider);
    // Rebuild when the active community changes (pubkey may differ).
    ref.watch(activeCommunityProvider);

    final nsec = relayConfig.nsec?.trim();
    if (nsec == null || nsec.isEmpty) {
      return const ChannelMutesState();
    }

    final pubkey = ref.watch(myPubkeyProvider);
    final crypto = ref.watch(channelMutesCryptoProvider);
    if (pubkey == null || crypto == null) {
      return const ChannelMutesState();
    }

    final prefs = ref.read(savedPrefsProvider);
    final signedRelay = SignedEventRelay(
      session: ref.read(relaySessionProvider.notifier),
      nsec: nsec,
    );

    late final ChannelMutesManager manager;
    manager = ChannelMutesManager(
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

    return ChannelMutesState(isReady: false, store: manager.store, version: 1);
  }

  void muteChannel(String channelId) => _manager?.muteChannel(channelId);

  void unmuteChannel(String channelId) => _manager?.unmuteChannel(channelId);

  void _emitManagerState(ChannelMutesManager manager) {
    if (_manager != manager) return;
    state = ChannelMutesState(
      isReady: _isInitialized,
      store: manager.store,
      version: state.version + 1,
    );
  }
}

final channelMutesProvider =
    NotifierProvider<ChannelMutesNotifier, ChannelMutesState>(
      ChannelMutesNotifier.new,
    );

/// Identity-scoped crypto survives connection status changes. A community or
/// signing-key change replaces it through [relayConfigProvider].
final channelMutesCryptoProvider = Provider<ChannelMutesCrypto?>((ref) {
  final config = ref.watch(relayConfigProvider);
  final pubkey = ref.watch(myPubkeyProvider);
  final nsec = config.nsec?.trim();
  if (nsec == null || nsec.isEmpty || pubkey == null) return null;
  try {
    return ChannelMutesCrypto(nsec, pubkey);
  } catch (_) {
    return null;
  }
});
