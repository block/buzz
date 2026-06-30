part of '../channel_detail_page.dart';

class _MessageList extends HookConsumerWidget {
  final List<MainTimelineEntry> entries;
  final List<TimelineMessage> allMessages;
  final String channelId;
  final String? currentPubkey;
  final bool isMember;
  final bool isArchived;

  const _MessageList({
    required this.entries,
    required this.allMessages,
    required this.channelId,
    required this.currentPubkey,
    required this.isMember,
    required this.isArchived,
  });

  static const _fetchOlderThreshold = 200.0;
  static const _latestThreshold = 48.0;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    // Pagination: fetch older messages when scrolling near the top.
    final scrollController = useScrollController();
    final isLoadingOlder = useState(false);
    final isAtLatest = useState(true);
    final latestEntryId = entries.isEmpty ? null : entries.last.message.id;
    final previousLatestEntryId = useRef<String?>(null);

    bool nearLatest() {
      if (!scrollController.hasClients) return true;
      return scrollController.position.pixels <= _latestThreshold;
    }

    void updateLatestState() {
      final next = nearLatest();
      if (isAtLatest.value != next) {
        isAtLatest.value = next;
      }
    }

    Future<void> scrollToLatest() async {
      if (!scrollController.hasClients) return;
      await scrollController.animateTo(
        0,
        duration: const Duration(milliseconds: 220),
        curve: Curves.easeOutCubic,
      );
      if (context.mounted) {
        isAtLatest.value = true;
      }
    }

    useEffect(() {
      void onScroll() {
        updateLatestState();
        if (isLoadingOlder.value) return;
        final notifier = ref.read(channelMessagesProvider(channelId).notifier);
        if (notifier.reachedOldest) return;
        // In a reversed ListView, maxScrollExtent is the oldest messages.
        final pos = scrollController.position;
        if (pos.pixels >= pos.maxScrollExtent - _fetchOlderThreshold) {
          isLoadingOlder.value = true;
          notifier.fetchOlder().whenComplete(
            () => isLoadingOlder.value = false,
          );
        }
      }

      scrollController.addListener(onScroll);
      return () => scrollController.removeListener(onScroll);
    }, [channelId, scrollController]);

    useEffect(() {
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (!context.mounted) return;
        updateLatestState();
      });
      return null;
    }, [entries.length, scrollController]);

    useEffect(() {
      final previous = previousLatestEntryId.value;
      previousLatestEntryId.value = latestEntryId;
      if (previous == null ||
          latestEntryId == null ||
          previous == latestEntryId ||
          !isAtLatest.value) {
        return null;
      }

      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (!context.mounted || !scrollController.hasClients) return;
        scrollController.animateTo(
          0,
          duration: const Duration(milliseconds: 220),
          curve: Curves.easeOutCubic,
        );
      });
      return null;
    }, [latestEntryId, scrollController]);

    if (entries.isEmpty) {
      return Center(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Icon(
              LucideIcons.messageSquare,
              size: Grid.xl,
              color: context.colors.onSurfaceVariant,
            ),
            const SizedBox(height: Grid.xxs),
            Text(
              'No messages yet',
              style: context.textTheme.bodyLarge?.copyWith(
                color: context.colors.onSurfaceVariant,
              ),
            ),
            const SizedBox(height: Grid.half),
            Text(
              'Be the first to say something!',
              style: context.textTheme.bodySmall?.copyWith(
                color: context.colors.onSurfaceVariant,
              ),
            ),
          ],
        ),
      );
    }

    // Build channel names map once for all message bubbles.
    final channelsAsync = ref.watch(channelsProvider);
    final channelNamesMap = <String, String>{};
    channelsAsync.whenData((channels) {
      for (final ch in channels) {
        channelNamesMap[ch.name.toLowerCase()] = ch.id;
      }
    });

    return Stack(
      children: [
        ListView.builder(
          key: const ValueKey('channel-message-list'),
          controller: scrollController,
          reverse: true,
          padding: EdgeInsets.only(
            left: Grid.gutter,
            right: Grid.gutter,
            top: frostedAppBarHeight(context),
            bottom: Grid.xxs,
          ),
          itemCount: entries.length + (isLoadingOlder.value ? 1 : 0),
          itemBuilder: (context, index) {
            // Loading indicator at the top (last index in reversed list).
            if (index >= entries.length) {
              return const Padding(
                padding: EdgeInsets.symmetric(vertical: Grid.xs),
                child: Center(
                  child: SizedBox(
                    width: 20,
                    height: 20,
                    child: CircularProgressIndicator(strokeWidth: 2),
                  ),
                ),
              );
            }

            // Reversed list: index 0 = newest (bottom of screen).
            final chronIdx = entries.length - 1 - index;
            final entry = entries[chronIdx];
            final message = entry.message;

            // Day boundary check — applies to all messages including system.
            final prevEntry = chronIdx > 0 ? entries[chronIdx - 1] : null;
            final prevMessage = prevEntry?.message;
            final showDayDivider =
                prevMessage == null ||
                !isSameDay(prevMessage.createdAt, message.createdAt);

            final showAuthor =
                !message.isSystem &&
                (prevMessage == null ||
                    prevMessage.isSystem ||
                    showDayDivider ||
                    prevMessage.pubkey.toLowerCase() !=
                        message.pubkey.toLowerCase() ||
                    (message.createdAt - prevMessage.createdAt) > 300);

            return Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                if (showDayDivider)
                  DayDivider(label: formatDayHeading(message.createdAt)),
                if (message.isSystem)
                  _SystemMessageRow(
                    message: message,
                    channelId: channelId,
                    currentPubkey: currentPubkey,
                    allMessages: null,
                    isMember: isMember,
                    isArchived: isArchived,
                  )
                else ...[
                  _MessageBubble(
                    message: message,
                    showAuthor: showAuthor,
                    channelNames: channelNamesMap,
                    currentChannelId: channelId,
                    currentPubkey: currentPubkey,
                    allMessages: allMessages,
                    isMember: isMember,
                    isArchived: isArchived,
                  ),
                  if (entry.summary != null)
                    _ThreadSummaryRow(
                      summary: entry.summary!,
                      message: message,
                      allMessages: allMessages,
                      channelId: channelId,
                      currentPubkey: currentPubkey,
                      isMember: isMember,
                      isArchived: isArchived,
                    ),
                ],
              ],
            );
          },
        ),
        if (!isAtLatest.value)
          Positioned(
            left: 0,
            right: 0,
            bottom: Grid.xs,
            child: Center(
              child: FilledButton.icon(
                key: const ValueKey('channel-jump-to-latest'),
                onPressed: scrollToLatest,
                style: FilledButton.styleFrom(
                  backgroundColor: context.colors.primaryContainer,
                  foregroundColor: context.colors.onPrimaryContainer,
                  padding: const EdgeInsets.symmetric(
                    horizontal: Grid.gutter,
                    vertical: Grid.xxs,
                  ),
                ),
                icon: const Icon(LucideIcons.arrowDown, size: 16),
                label: const Text('Latest'),
              ),
            ),
          ),
      ],
    );
  }
}
