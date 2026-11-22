import 'dart:async';
import 'dart:convert';
import 'package:flutter/widgets.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import '../../../shared/community/community_provider.dart';
import '../../../shared/relay/relay.dart';
import '../../../shared/theme/theme_provider.dart';
import '../channel_sort/channel_sort_provider.dart';
import '../channel_sections/channel_sections_provider.dart';
import '../channel_sort/channel_sort_storage.dart';
import 'channel_filter_manager.dart';
import 'channel_filter_store.dart';

final channelFilterProvider = Provider.autoDispose<ChannelFilterManager?>(
  (ref) => _createManager(ref, 'channel-filter'),
);

// Solo projects and Team sections each support 100 entries. Keep their
// preference budgets independent when both inventories are full.
final soloFilterProvider = Provider.autoDispose<ChannelFilterManager?>(
  (ref) => _createManager(ref, 'solo-channel-filter'),
);

ChannelFilterManager? _createManager(Ref ref, String coordinate) {
  // Identity and community scope only: push bookkeeping rewrites the active
  // community and the socket reconnects without changing either, and a new
  // manager would abandon an open section sheet.
  final config = ref.watch(relayConfigProvider);
  final pubkey = ref.watch(myPubkeyProvider);
  final crypto = ref.watch(channelSortCryptoProvider);
  final relayUrl = ref.watch(
    activeCommunityProvider.select((value) => value.value?.relayUrl),
  );
  final nsec = config.nsec?.trim();
  if (nsec == null ||
      nsec.isEmpty ||
      pubkey == null ||
      crypto == null ||
      relayUrl == null) {
    return null;
  }
  final prefs = ref.read(savedPrefsProvider);
  final cacheKey =
      'buzz.$coordinate.v1:$pubkey:${Uri.encodeComponent(normalizeChannelSortRelayUrl(relayUrl))}';
  ChannelFilterStore? cached;
  try {
    final raw = prefs.getString(cacheKey);
    if (raw != null) {
      cached = ChannelFilterStore.parse(
        jsonDecode(raw) as Map<String, dynamic>,
      );
    }
  } catch (_) {
    /* A malformed display cache never grants write authority. */
  }
  final session = ref.read(relaySessionProvider.notifier);
  final signer = SignedEventRelay(session: session, nsec: nsec);
  final manager = ChannelFilterManager(
    coordinate: coordinate,
    isGroupLive: (group) =>
        !group.startsWith('section:') ||
        ref.read(sidebarLayoutProvider).section(group.substring(8)) != null,
    pubkey: pubkey,
    crypto: crypto,
    cached: cached,
    fetch: () => session.fetchHistory(
      NostrFilter(
        kinds: const [EventKind.readState],
        authors: [pubkey],
        tags: {
          '#d': [coordinate],
        },
        limit: 1,
      ),
    ),
    publish: (content, createdAt) async {
      await signer.submit(
        kind: EventKind.readState,
        content: content,
        tags: [
          ['d', coordinate],
          ['t', coordinate],
        ],
        createdAt: createdAt,
      );
    },
    cache: (store) {
      unawaited(prefs.setString(cacheKey, jsonEncode(store.toJson())));
    },
  );
  bool connected() =>
      ref.read(relaySessionProvider).status == SessionStatus.connected;
  if (connected()) scheduleMicrotask(manager.refresh);
  ref.listen(
    relaySessionProvider.select((s) => s.status == SessionStatus.connected),
    (_, now) {
      if (now) unawaited(manager.refresh());
    },
  );
  ref.listen(appLifecycleProvider, (_, next) {
    if (next == AppLifecycleState.resumed && connected()) {
      unawaited(manager.refresh());
    }
  });
  ref.onDispose(manager.dispose);
  return manager;
}
