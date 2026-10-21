import 'dart:async';
import 'dart:convert';
import 'package:flutter/widgets.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;
import '../../../shared/community/community_provider.dart';
import '../../../shared/relay/relay.dart';
import '../../../shared/theme/theme_provider.dart';
import '../channel_sort/channel_sort_manager.dart';
import '../channel_sections/channel_sections_provider.dart';
import '../channel_sort/channel_sort_storage.dart';
import 'channel_filter_manager.dart';
import 'channel_filter_store.dart';

final channelFilterProvider = Provider.autoDispose<ChannelFilterManager?>((
  ref,
) {
  final config = ref.watch(relayConfigProvider);
  final connected = ref.watch(
    relaySessionProvider.select((s) => s.status == SessionStatus.connected),
  );
  final community = ref.watch(activeCommunityProvider).unwrapPrevious().value;
  final nsec = config.nsec?.trim();
  if (nsec == null || nsec.isEmpty || community == null) return null;
  final prefs = ref.read(savedPrefsProvider);
  final pubkey = nostr.Keys(nostr.Nip19.decode(payload: nsec).data).public;
  final cacheKey =
      'buzz.channel-filter.v1:$pubkey:${Uri.encodeComponent(normalizeChannelSortRelayUrl(community.relayUrl))}';
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
    isGroupLive: (group) =>
        !group.startsWith('section:') ||
        ref
            .read(channelSectionsProvider)
            .store
            .sections
            .any((s) => 'section:${s.id}' == group),
    pubkey: pubkey,
    crypto: ChannelSortCrypto(nsec, pubkey),
    cached: cached,
    fetch: () => session.fetchHistory(
      NostrFilter(
        kinds: const [EventKind.readState],
        authors: [pubkey],
        tags: const {
          '#d': ['channel-filter'],
        },
        limit: 1,
      ),
    ),
    publish: (content, createdAt) async {
      await signer.submit(
        kind: EventKind.readState,
        content: content,
        tags: const [
          ['d', 'channel-filter'],
          ['t', 'channel-filter'],
        ],
        createdAt: createdAt,
      );
    },
    cache: (store) {
      unawaited(prefs.setString(cacheKey, jsonEncode(store.toJson())));
    },
  );
  if (connected) scheduleMicrotask(manager.refresh);
  ref.listen(appLifecycleProvider, (_, next) {
    if (connected && next == AppLifecycleState.resumed) {
      unawaited(manager.refresh());
    }
  });
  ref.onDispose(manager.dispose);
  return manager;
});
