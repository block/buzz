import 'dart:async';

import 'package:hooks_riverpod/hooks_riverpod.dart';
import '../../../shared/me/me_sync.dart';
import '../../../shared/read_state/read_state_provider.dart';
import '../../../shared/read_state/read_state_time.dart';
import '../../../shared/relay/relay.dart';
import '../channel_unread.dart';
import '../channels_provider.dart';

/// Solo uses Home's initial read baseline even before the tab is visited.
final meUnreadChannelIdsProvider =
    NotifierProvider<MeUnreadChannelIds, Set<String>>(MeUnreadChannelIds.new);

/// Seeds initial history once per channel and identity before displaying unread.
class MeUnreadChannelIds extends Notifier<Set<String>> {
  @override
  Set<String> build() {
    final organization = ref.watch(meGroupsProvider).value;
    final channels = ref.watch(channelsProvider).value ?? [];
    final reads = ref.watch(readStateProvider);
    final seeds = ref.watch(_initialSeedsProvider);
    final baselines = seeds.baselines;
    final seeded = seeds.completed;
    if (reads.pubkey == null || organization == null) return const {};
    final active = channels
        .where(
          (c) =>
              c.isMember &&
              !c.isArchived &&
              c.isPersonalChat(reads.pubkey, organization.channels),
        )
        .toList();
    final pending = active.where((c) => !seeded.contains(c.id)).toList();
    for (final channel in pending) {
      // Capture before deferring: a newer event must not move this baseline.
      baselines.putIfAbsent(
        channel.id,
        () => dateTimeToUnixSeconds(channel.lastMessageAt),
      );
    }
    if (!reads.isReady) return const {};
    var disposed = false;
    ref.onDispose(() => disposed = true);
    if (pending.isNotEmpty) {
      scheduleMicrotask(() {
        if (disposed) return;
        final notifier = ref.read(readStateProvider.notifier);
        for (final channel in pending) {
          final baseline = baselines[channel.id];
          if (ref.read(readStateProvider).effectiveTimestamp(channel.id) ==
                  null &&
              baseline != null) {
            notifier.seedContextRead(channel.id, baseline);
          }
          // Empty chats also complete initialization; their first event is new.
          seeded.add(channel.id);
        }
        state = computeUnreadChannelIds(
          channels: active,
          readState: ref.read(readStateProvider),
          channelsNotifier: ref.read(channelsProvider.notifier),
        );
      });
    }
    return computeUnreadChannelIds(
      channels: active.where(
        (c) => seeded.contains(c.id) || reads.effectiveTimestamp(c.id) != null,
      ),
      readState: reads,
      channelsNotifier: ref.read(channelsProvider.notifier),
    );
  }
}

// Retain the captured baseline across channel/read-marker rebuilds, but never
// across an identity or community boundary. Readiness must preserve history.
final _initialSeedsProvider = Provider((ref) {
  ref.watch(readStateProvider.select((s) => s.pubkey));
  ref.watch(relayConfigProvider.select((c) => c.baseUrl));
  return _InitialSeeds();
});

class _InitialSeeds {
  final baselines = <String, int?>{};
  final completed = <String>{};
}
