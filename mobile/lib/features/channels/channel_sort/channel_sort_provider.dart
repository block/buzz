import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;

import '../../../shared/relay/relay.dart';
import '../../../shared/theme/theme_provider.dart';
import '../../../shared/community/community_provider.dart';
import 'channel_sort_manager.dart';
import 'channel_sort_storage.dart';

class ChannelSortState {
  final bool isReady;
  final ChannelSortStore store;

  /// Bumped on every change to force downstream rebuilds.
  final int version;

  const ChannelSortState({
    this.isReady = false,
    this.store = const ChannelSortStore(),
    this.version = 0,
  });

  ChannelSortMode sortModeFor(String groupKey) =>
      store.groups[groupKey] ?? kDefaultSortMode;
}

class ChannelSortNotifier extends Notifier<ChannelSortState> {
  ChannelSortManager? _manager;

  @override
  ChannelSortState build() {
    _manager?.dispose();
    _manager = null;

    final relayConfig = ref.watch(relayConfigProvider);
    final sessionState = ref.watch(relaySessionProvider);
    final activeCommunity = ref.watch(activeCommunityProvider).value;

    final nsec = relayConfig.nsec?.trim();
    if (nsec == null || nsec.isEmpty) {
      return const ChannelSortState();
    }

    final pubkey = _safePubkeyFromNsec(nsec);
    if (pubkey == null || pubkey.isEmpty) {
      return const ChannelSortState();
    }

    final ChannelSortCrypto crypto;
    try {
      crypto = ChannelSortCrypto(nsec, pubkey);
    } catch (_) {
      return const ChannelSortState();
    }

    final relayUrl = activeCommunity?.relayUrl.trim();
    if (relayUrl == null || relayUrl.isEmpty) {
      return const ChannelSortState();
    }

    final prefs = ref.read(savedPrefsProvider);
    final signedRelay = SignedEventRelay(
      session: ref.read(relaySessionProvider.notifier),
      nsec: nsec,
    );

    late final ChannelSortManager manager;
    manager = ChannelSortManager(
      pubkey: pubkey,
      relayUrl: relayUrl,
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

    return ChannelSortState(isReady: false, store: manager.store, version: 1);
  }

  void setSortModeFor(
    String groupKey,
    ChannelSortMode mode, {
    Iterable<String>? liveSectionIds,
  }) =>
      _manager?.setSortModeFor(groupKey, mode, liveSectionIds: liveSectionIds);

  void _emitManagerState(ChannelSortManager manager) {
    if (_manager != manager) return;
    state = ChannelSortState(
      isReady: true,
      store: manager.store,
      version: state.version + 1,
    );
  }
}

final channelSortProvider =
    NotifierProvider<ChannelSortNotifier, ChannelSortState>(
      ChannelSortNotifier.new,
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
