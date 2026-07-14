part of '../channel_detail_page.dart';

class _MessageList extends HookConsumerWidget {
  final List<MainTimelineEntry> entries;
  final List<TimelineMessage> allMessages;
  final String? initialMessageId;
  final String? initialThreadRootId;
  final String channelId;
  final String? currentPubkey;
  final bool isMember;
  final bool isArchived;

  const _MessageList({
    required this.entries,
    required this.allMessages,
    required this.initialMessageId,
    required this.initialThreadRootId,
    required this.channelId,
    required this.currentPubkey,
    required this.isMember,
    required this.isArchived,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final itemScrollController = useMemoized(ItemScrollController.new);
    final itemPositionsListener = useMemoized(ItemPositionsListener.create);
    final isLoadingOlder = useState(false);
    final isAtLatest = useState(true);
    final hasUserScrolled = useState(false);
    final latestEntryId = entries.isEmpty ? null : entries.last.message.id;
    final previousLatestEntryId = useRef<String?>(null);
    final didOpenInitialThread = useRef(false);
    final didJumpToInitialMessage = useRef(false);

    int? reversedIndexOf(String? messageId) {
      if (messageId == null) return null;
      final chronologicalIndex = entries.indexWhere(
        (entry) => entry.message.id == messageId,
      );
      return chronologicalIndex < 0
          ? null
          : entries.length - 1 - chronologicalIndex;
    }

    Future<void> scrollToLatest() async {
      if (!itemScrollController.isAttached) return;
      await itemScrollController.scrollTo(
        index: 0,
        duration: const Duration(milliseconds: 220),
        curve: Curves.easeOutCubic,
      );
      if (context.mounted) isAtLatest.value = true;
    }

    useEffect(() {
      void onPositionsChanged() {
        final positions = itemPositionsListener.itemPositions.value;
        if (positions.isEmpty) return;
        final nextIsAtLatest = positions.any(
          (position) => position.index == 0 && position.itemLeadingEdge < 1,
        );
        if (isAtLatest.value != nextIsAtLatest) {
          isAtLatest.value = nextIsAtLatest;
        }

        final oldestVisible = positions
            .map((position) => position.index)
            .reduce((a, b) => a > b ? a : b);
        if (!hasUserScrolled.value ||
            oldestVisible < entries.length - 3 ||
            isLoadingOlder.value) {
          return;
        }
        final notifier = ref.read(channelMessagesProvider(channelId).notifier);
        if (notifier.reachedOldest) return;
        isLoadingOlder.value = true;
        notifier.fetchOlder().whenComplete(() => isLoadingOlder.value = false);
      }

      itemPositionsListener.itemPositions.addListener(onPositionsChanged);
      return () => itemPositionsListener.itemPositions.removeListener(
        onPositionsChanged,
      );
    }, [channelId, entries.length, itemPositionsListener]);

    useEffect(() {
      if (initialThreadRootId == null || didOpenInitialThread.value) {
        return null;
      }
      final threadHead = allMessages
          .where((message) => message.id == initialThreadRootId)
          .firstOrNull;
      if (threadHead == null) return null;
      didOpenInitialThread.value = true;
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (!context.mounted) return;
        Navigator.of(context).push(
          MaterialPageRoute<void>(
            builder: (_) => ThreadDetailPage(
              threadHead: threadHead,
              allMessages: allMessages,
              channelId: channelId,
              currentPubkey: currentPubkey,
              isMember: isMember,
              isArchived: isArchived,
              initialMessageId: initialMessageId,
            ),
          ),
        );
      });
      return null;
    }, [initialThreadRootId, allMessages]);

    useEffect(() {
      final targetIndex = reversedIndexOf(initialMessageId);
      if (initialThreadRootId != null ||
          targetIndex == null ||
          didJumpToInitialMessage.value) {
        return null;
      }
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (!context.mounted || !itemScrollController.isAttached) return;
        itemScrollController.jumpTo(index: targetIndex, alignment: 0.35);
        didJumpToInitialMessage.value = true;
      });
      return null;
    }, [initialMessageId, initialThreadRootId, entries.length]);

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
        if (context.mounted) scrollToLatest();
      });
      return null;
    }, [latestEntryId]);

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
        NotificationListener<ScrollNotification>(
          onNotification: (notification) {
            if (notification is ScrollStartNotification &&
                notification.dragDetails != null) {
              hasUserScrolled.value = true;
            }
            return false;
          },
          child: ScrollablePositionedList.builder(
            key: const ValueKey('channel-message-list'),
            itemScrollController: itemScrollController,
            itemPositionsListener: itemPositionsListener,
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
