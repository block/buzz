import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../../shared/relay/relay.dart';
import '../../../shared/theme/theme_provider.dart';
import '../../../shared/community/community_provider.dart';
import 'channel_mutes_manager.dart';
import 'channel_mutes_storage.dart';

class ChannelMutesState {
  final bool isReady;
  final ChannelMuteStore store;

  /// Relay-confirmed mutes used to suppress channel row dots.
  final Set<String> confirmedMutedChannelIds;

  /// Bumped on every change to force downstream rebuilds.
  final int version;

  const ChannelMutesState({
    this.isReady = false,
    this.store = const ChannelMuteStore(),
    this.confirmedMutedChannelIds = const {},
    this.version = 0,
  });
}

class ChannelMutesNotifier extends Notifier<ChannelMutesState> {
  ChannelMutesManager? _manager;
  bool _isInitialized = false;
  (String, String)? _confirmationScope;
  var _confirmedMutes = const ConfirmedChannelMutes.empty();

  @override
  ChannelMutesState build() {
    _manager?.dispose();
    _manager = null;
    _isInitialized = false;

    final relayConfig = ref.watch(relayConfigProvider);
    final sessionState = ref.watch(relaySessionProvider);
    // Rebuild when the active community changes (pubkey may differ).
    ref.watch(activeCommunityProvider);

    final nsec = relayConfig.nsec?.trim();
    final pubkey = ref.watch(myPubkeyProvider);
    final crypto = ref.watch(channelMutesCryptoProvider);
    if (nsec == null || nsec.isEmpty || pubkey == null || crypto == null) {
      _confirmationScope = null;
      _confirmedMutes = const ConfirmedChannelMutes.empty();
      return const ChannelMutesState();
    }
    final scope = (pubkey, relayConfig.baseUrl);
    if (_confirmationScope != scope) {
      _confirmationScope = scope;
      _confirmedMutes = const ConfirmedChannelMutes.empty();
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
    ref.listen(appForegroundProvider, (_, foreground) {
      if (foreground) manager.refreshFromRelay();
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

    return ChannelMutesState(
      store: manager.store,
      confirmedMutedChannelIds: _confirmedMutes.channelIds,
      version: 1,
    );
  }

  void muteChannel(String channelId) => _manager?.muteChannel(channelId);

  void unmuteChannel(String channelId) => _manager?.unmuteChannel(channelId);

  void _emitManagerState(ChannelMutesManager manager) {
    if (_manager != manager) return;
    // Keep the last confirmed view while a replacement manager reconnects.
    // The manager still reads and merges its new session's store normally.
    final confirmation = manager.confirmedMutes;
    if (_confirmedMutes.precedes(
      confirmation.createdAt,
      confirmation.eventId,
    )) {
      _confirmedMutes = confirmation;
    }
    state = ChannelMutesState(
      isReady: _isInitialized,
      store: manager.store,
      confirmedMutedChannelIds: _confirmedMutes.channelIds,
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
