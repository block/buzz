part of '../channel_detail_page.dart';

class _HuddleHistoryAuthorization {
  bool allowed = false;
}

// The lease lives only while this history route is open, and is retired on
// identity/community changes. It is never a persisted permission cache.
final _huddleHistoryAuthorizationProvider = Provider.autoDispose
    .family<_HuddleHistoryAuthorization, String>((ref, channelId) {
      ref.watch(relayConfigProvider);
      ref.watch(myPubkeyProvider);
      return _HuddleHistoryAuthorization();
    });

/// Read the original room, where voice transcripts and Huddle chat are stored.
/// Opening history never joins audio or changes room membership.
final _huddleHistoryAccessProvider = FutureProvider.autoDispose
    .family<bool, String>((ref, channelId) async {
      ref.watch(relayConfigProvider);
      ref.watch(myPubkeyProvider);
      ref.watch(
        channelMembershipUpdateProvider(
          channelId,
        ).select((update) => update.version),
      );
      final authorization = ref.watch(
        _huddleHistoryAuthorizationProvider(channelId),
      );
      final membership = ref.watch(huddleHistoryMembershipProvider(channelId));
      if (membership == false) {
        authorization.allowed = false;
        return false;
      }
      // A known member reads the room directly, as before this route checked
      // access, so cached rows stay available offline. A later refusal still
      // clears them in the message provider.
      if (membership == true) {
        authorization.allowed = true;
        return true;
      }
      // Keep this open route's successful authorization through a transient
      // disconnect. Reopening still requires a fresh check; reconnecting
      // revalidates before showing any retained private history.
      ref.listen(relaySessionProvider, (previous, next) {
        if (next.status == SessionStatus.connected &&
            previous?.status != SessionStatus.connected) {
          ref.invalidateSelf();
        }
      });
      final status = ref.read(relaySessionProvider).status;
      if (status != SessionStatus.connected) {
        if (authorization.allowed) return true;
        throw StateError('Not connected');
      }
      try {
        await ref
            .read(relaySessionProvider.notifier)
            .fetchHistory(NostrFilters.messages(channelId, limit: 1));
        authorization.allowed = true;
        return true;
      } catch (error) {
        if (isRelayAccessDenied(error)) {
          authorization.allowed = false;
          return false;
        }
        if (authorization.allowed) return true;
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
          relayHistoryReady: ref.read(provider.notifier).hasLoadedHistory,
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
