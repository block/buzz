import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/relay/relay.dart';
import 'channel_event_order.dart';
import 'channel_messages_provider.dart';
import 'channel_window.dart';
import 'me/me_messages_provider.dart';
import 'saved_channel_page.dart';

typedef _ChannelTimeline = ({
  AsyncValue<List<NostrEvent>> messages,
  Map<String, ChannelWindowThreadSummary> summaries,
  bool relayHistoryReady,
  bool isShowingPreview,
  bool previewRefreshFailed,
});

final channelTimelineProjectionProvider = Provider.autoDispose
    .family<
      _ChannelTimeline,
      ({String channelId, bool personal, bool allowPreview})
    >((ref, args) {
      final provider = args.personal
          ? meMessagesProvider(args.channelId)
          : channelMessagesProvider(args.channelId);
      final relay = ref.watch(provider);
      final notifier = ref.read(provider.notifier);
      final relayHistoryReady = notifier.hasLoadedHistory;
      final error = relay.error;
      final preview =
          args.allowPreview &&
              !relayHistoryReady &&
              (error == null || !isRelayChannelAccessDenied(error))
          ? ref
                .watch(savedChannelPreviewProvider(args.channelId))
                .unwrapPrevious()
                .value
          : null;
      final current = relay.value ?? const <NostrEvent>[];
      final saved = preview?.events;
      final messages = switch (saved) {
        null => relay,
        _ when current.isEmpty => AsyncData(saved),
        _ => AsyncData(
          ({
            for (final event in saved) event.id: event,
            for (final event in current) event.id: event,
          }).values.toList()..sort(compareChannelTimelineEventsChronologically),
        ),
      };
      final liveSummaries = notifier.threadSummaries;
      return (
        messages: messages,
        summaries: preview == null
            ? liveSummaries
            : {...preview.summaries, ...liveSummaries},
        relayHistoryReady: relayHistoryReady,
        isShowingPreview: preview != null,
        previewRefreshFailed: preview != null && notifier.historyRefreshFailed,
      );
    });
