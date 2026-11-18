part of '../channel_detail_page.dart';

/// Read the original room, where voice transcripts and Huddle chat are stored.
/// Opening history never joins audio or changes room membership.
class _HuddleHistoryPage extends HookConsumerWidget {
  const _HuddleHistoryPage({required this.channelId});
  final String channelId;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final provider = channelMessagesProvider(channelId);
    final messages = ref.watch(provider);
    final currentPubkey = ref.watch(currentPubkeyProvider);
    final dateScrollActivity = useDateHeaderScrollActivity(channelId);
    final stickyDateHeaderState = useValueNotifier(
      StickyDateHeaderState.hidden,
    );
    return FrostedScaffold(
      appBar: const FrostedAppBar(title: Text('Huddle')),
      body: messages.when(
        loading: () => const Center(child: BuzzLoadingIndicator()),
        error: (_, _) => LoadErrorView(
          message: 'Couldn’t load this Huddle conversation.',
          onRetry: () => ref.invalidate(provider),
        ),
        data: (events) {
          final timeline = formatTimeline(
            events,
            currentPubkey: currentPubkey,
          ).where((message) => !message.isSystem).toList();
          if (timeline.isEmpty) {
            return const Center(child: Text('No messages in this Huddle.'));
          }
          return _MessageList(
            dateScrollActivity: dateScrollActivity,
            stickyDateHeaderState: stickyDateHeaderState,
            entries: buildMainTimelineEntries(
              timeline,
              relaySummaries: ref.read(provider.notifier).threadSummaries,
            ),
            allMessages: timeline,
            initialMessageId: null,
            initialThreadRootId: null,
            initialThreadRouteBehavior: InitialThreadRouteBehavior.push,
            initialOrdinaryUnreadMessageIds: const {},
            initialOldestOrdinaryUnreadMessageId: null,
            initialForcedUnreadMessageIds: const {},
            hasInitialUnread: false,
            channelId: channelId,
            notifier: ref.read(provider.notifier),
            isDm: false,
            flat: false,
            currentPubkey: currentPubkey,
            isMember: false,
            isArchived: true,
            appBarTitleContentHeight: 0,
            composerBottomInset: 0,
          );
        },
      ),
    );
  }
}
