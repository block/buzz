import 'package:hooks_riverpod/hooks_riverpod.dart';
import '../channels_provider.dart';

/// When anyone last posted in a channel, for the Active filter. Loading until
/// the channel list's latest-message read covers the channel; data (null when
/// it has no messages) after that. It comes from the list's own reads and
/// live delivery, so it sends no relay requests of its own. While watched, the
/// reconnect catch-up also reads the channel's activity gap.
final channelFilterActivityProvider = Provider.autoDispose
    .family<AsyncValue<int?>, String>((ref, channelId) {
      ref.watch(_activityWatchProvider(channelId));
      final notifier = ref.watch(channelsProvider.notifier);
      final (:known, :at) = ref.watch(
        channelsProvider.select((_) => notifier.activityFor(channelId)),
      );
      return known ? AsyncData(at) : const AsyncLoading();
    });

/// Holds the Active section's interest in a channel for as long as anything
/// watches its activity, across the rebuilds each activity change causes.
final _activityWatchProvider = Provider.autoDispose.family<void, String>((
  ref,
  channelId,
) {
  ref.onDispose(ref.watch(channelsProvider.notifier).watchActivity(channelId));
});
