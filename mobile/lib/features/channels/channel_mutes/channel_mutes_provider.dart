import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;

import '../../../shared/relay/relay.dart';
import '../../../shared/theme/theme_provider.dart';
import '../../../shared/workspace/workspace_provider.dart';
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

  @override
  ChannelMutesState build() {
    _manager?.dispose(flushPending: false);
    _manager = null;

    final relayConfig = ref.watch(relayConfigProvider);
    final sessionState = ref.watch(relaySessionProvider);
    // Rebuild when the active workspace changes (pubkey may differ).
    ref.watch(activeWorkspaceProvider);

    final nsec = relayConfig.nsec?.trim();
    if (nsec == null || nsec.isEmpty) {
      return const ChannelMutesState();
    }

    final pubkey = _safePubkeyFromNsec(nsec);
    if (pubkey == null || pubkey.isEmpty) {
      return const ChannelMutesState();
    }

    final ChannelMutesCrypto crypto;
    try {
      crypto = ChannelMutesCrypto(nsec, pubkey);
    } catch (_) {
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
      prefs: prefs,
      crypto: crypto,
      relaySession: ref.read(relaySessionProvider.notifier),
      signedEventRelay: signedRelay,
      remoteEnabled: sessionState.status == SessionStatus.connected,
      onChanged: () => _emitManagerState(manager),
    );
    _manager = manager;

    ref.onDispose(() {
      manager.dispose();
      if (_manager == manager) {
        _manager = null;
      }
    });

    Future.microtask(() async {
      await manager.initialize();
      if (_manager != manager) return;
      _emitManagerState(manager);
    });

    return ChannelMutesState(isReady: false, store: manager.store, version: 1);
  }

  void muteChannel(String channelId) => _manager?.muteChannel(channelId);

  void unmuteChannel(String channelId) => _manager?.unmuteChannel(channelId);

  void _emitManagerState(ChannelMutesManager manager) {
    if (_manager != manager) return;
    state = ChannelMutesState(
      isReady: true,
      store: manager.store,
      version: state.version + 1,
    );
  }
}

final channelMutesProvider =
    NotifierProvider<ChannelMutesNotifier, ChannelMutesState>(
      ChannelMutesNotifier.new,
    );

String? _safePubkeyFromNsec(String nsec) {
  try {
    final privkeyHex = nostr.Nip19.decode(payload: nsec).data;
    if (privkeyHex.isEmpty) return null;
    return nostr.Keys(privkeyHex).public;
  } catch (_) {
    return null;
  }
}
