part of '../channel_detail_page.dart';

/// Read the original room, where voice transcripts and Huddle chat are stored.
/// Opening history never joins audio or changes room membership.
final _huddleHistoryAccessProvider = FutureProvider.autoDispose
    .family<bool, String>((ref, channelId) async {
      ref.watch(relayConfigProvider);
      ref.watch(myPubkeyProvider);
      ref.watch(channelMembershipUpdateProvider(channelId));
      final membership = ref.watch(huddleHistoryMembershipProvider(channelId));
      if (membership == false) return false;
      final status = ref.watch(relaySessionProvider.select((s) => s.status));
      if (status != SessionStatus.connected) throw StateError('Not connected');
      try {
        await ref
            .read(relaySessionProvider.notifier)
            .fetchHistory(NostrFilters.messages(channelId, limit: 1));
        return true;
      } catch (error) {
        if (isRelayAccessDenied(error)) return false;
        rethrow;
      }
    });

class _HuddleHistoryPage extends ConsumerWidget {
  const _HuddleHistoryPage({required this.channelId});
  final String channelId;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final access = ref.watch(_huddleHistoryAccessProvider(channelId));
    return FrostedScaffold(
      appBar: const FrostedAppBar(title: Text('Huddle')),
      body: access.when(
        skipLoadingOnRefresh: false,
        loading: () => const Center(child: BuzzLoadingIndicator()),
        error: (error, _) => isRelayAccessDenied(error)
            ? const Center(
                child: Text('You no longer have access to this Huddle.'),
              )
            : LoadErrorView(
                message: 'Couldn’t load this Huddle conversation.',
                onRetry: () =>
                    ref.invalidate(_huddleHistoryAccessProvider(channelId)),
              ),
        data: (allowed) => allowed
            ? _HuddleHistoryMessages(channelId: channelId)
            : const Center(
                child: Text('You no longer have access to this Huddle.'),
              ),
      ),
    );
  }
}

class _HuddleHistoryMessages extends HookConsumerWidget {
  const _HuddleHistoryMessages({required this.channelId});
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
    return messages.when(
      loading: () => const Center(child: BuzzLoadingIndicator()),
      error: (error, _) => isRelayAccessDenied(error)
          ? const Center(
              child: Text('You no longer have access to this Huddle.'),
            )
          : LoadErrorView(
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
    );
  }
}
