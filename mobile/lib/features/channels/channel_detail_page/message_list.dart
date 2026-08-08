part of '../channel_detail_page.dart';

class _MessageList extends HookConsumerWidget {
  final List<MainTimelineEntry> entries;
  final List<TimelineMessage> allMessages;
  final String? initialMessageId;
  final String? initialThreadRootId;
  final Set<String> initialOrdinaryUnreadMessageIds;
  final String? initialOldestOrdinaryUnreadMessageId;
  final Set<String> initialForcedUnreadMessageIds;
  final bool hasInitialUnread;
  final String channelId;
  final String? currentPubkey;
  final bool isMember;
  final bool isArchived;
  final double appBarTitleContentHeight;
  final double composerBottomInset;

  const _MessageList({
    required this.entries,
    required this.allMessages,
    required this.initialMessageId,
    required this.initialThreadRootId,
    required this.initialOrdinaryUnreadMessageIds,
    required this.initialOldestOrdinaryUnreadMessageId,
    required this.initialForcedUnreadMessageIds,
    required this.hasInitialUnread,
    required this.channelId,
    required this.currentPubkey,
    required this.isMember,
    required this.isArchived,
    required this.appBarTitleContentHeight,
    required this.composerBottomInset,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final displayEntries = groupMembershipTimelineEntries(entries);
    final itemScrollController = useMemoized(ItemScrollController.new);
    final itemPositionsListener = useMemoized(ItemPositionsListener.create);
    final isLoadingOlder = useState(false);
    final isAtLatest = useState(true);
    final hasUserScrolled = useState(false);
    final followsLatest = useRef(
      initialMessageId == null && initialThreadRootId == null,
    );
    final isAutoScrolling = useRef(false);
    final latestRealignmentQueued = useRef(false);
    final latestEntryId = entries.isEmpty ? null : entries.last.message.id;
    final previousLatestEntryId = useRef<String?>(null);
    final didOpenInitialThread = useRef(false);
    final didJumpToInitialMessage = useRef(false);
    final isUnreadNavigationDismissed = useState(false);
    final detachedWhileUnreadShown = useRef(false);
    final oldestUnreadMessageId = useState<String?>(null);
    final unreadBoundaryLoadFailed = useState(false);
    final unreadBoundaryFetchCount = useRef(0);
    final hasUnreadDeepLink =
        initialMessageId != null || initialThreadRootId != null;
    final notifier = ref.read(channelMessagesProvider(channelId).notifier);

    useEffect(
      () {
        if (!hasInitialUnread ||
            hasUnreadDeepLink ||
            oldestUnreadMessageId.value != null ||
            unreadBoundaryLoadFailed.value ||
            entries.isEmpty) {
          return null;
        }

        final hasLoadedOrdinaryTarget =
            initialOldestOrdinaryUnreadMessageId != null &&
            entries.any(
              (entry) =>
                  entry.message.id == initialOldestOrdinaryUnreadMessageId,
            );
        final hasLoadedForcedTarget = entries.any(
          (entry) => initialForcedUnreadMessageIds.contains(entry.message.id),
        );
        final hasKnownTarget =
            initialOldestOrdinaryUnreadMessageId != null ||
            initialForcedUnreadMessageIds.isNotEmpty;
        final hasLoadedFetchTarget =
            initialOldestOrdinaryUnreadMessageId != null
            ? hasLoadedOrdinaryTarget
            : hasLoadedForcedTarget;
        final canFetchTarget =
            hasKnownTarget &&
            !hasLoadedFetchTarget &&
            !notifier.reachedOldest &&
            unreadBoundaryFetchCount.value < 4;
        if (canFetchTarget) {
          unreadBoundaryFetchCount.value += 1;
          var cancelled = false;
          unawaited(
            Future<void>(() async {
              final loaded = await notifier.fetchOlder();
              if (!cancelled && !loaded && !notifier.reachedOldest) {
                unreadBoundaryLoadFailed.value = true;
              }
            }),
          );
          return () => cancelled = true;
        }

        if (hasKnownTarget &&
            !hasLoadedFetchTarget &&
            !notifier.reachedOldest) {
          unreadBoundaryLoadFailed.value = true;
        }

        final ordinaryUnread = entries
            .where(
              (entry) =>
                  initialOrdinaryUnreadMessageIds.contains(entry.message.id),
            )
            .map((entry) => entry.message)
            .firstOrNull;
        final forcedUnread = entries
            .where(
              (entry) =>
                  initialForcedUnreadMessageIds.contains(entry.message.id),
            )
            .map((entry) => entry.message)
            .firstOrNull;
        final candidates = [ordinaryUnread, forcedUnread].nonNulls.toList()
          ..sort((a, b) => a.createdAt.compareTo(b.createdAt));
        oldestUnreadMessageId.value = candidates.firstOrNull?.id;
        return null;
      },
      [
        hasInitialUnread,
        hasUnreadDeepLink,
        initialOrdinaryUnreadMessageIds,
        initialOldestOrdinaryUnreadMessageId,
        initialForcedUnreadMessageIds,
        entries.length,
        notifier.reachedOldest,
        unreadBoundaryLoadFailed.value,
      ],
    );

    final showUnreadNavigation =
        !isUnreadNavigationDismissed.value &&
        oldestUnreadMessageId.value != null;

    int? reversedIndexOf(String? messageId) {
      if (messageId == null) return null;
      final chronologicalIndex = displayEntries.indexWhere(
        (group) => group.any((entry) => entry.message.id == messageId),
      );
      return chronologicalIndex < 0
          ? null
          : displayEntries.length - 1 - chronologicalIndex;
    }

    double latestAlignment() {
      final viewportHeight = context.size?.height ?? 0;
      return viewportHeight > 0
          ? (composerBottomInset / viewportHeight).clamp(0.0, 1.0).toDouble()
          : 0.0;
    }

    Future<void> scrollToLatest() async {
      if (!itemScrollController.isAttached || isAutoScrolling.value) return;
      followsLatest.value = true;
      hasUserScrolled.value = false;
      isAutoScrolling.value = true;
      try {
        await itemScrollController.scrollTo(
          index: 0,
          alignment: latestAlignment(),
          duration: const Duration(milliseconds: 220),
          curve: Curves.easeOutCubic,
        );
        if (context.mounted && !hasUserScrolled.value) {
          isAtLatest.value = true;
        }
      } finally {
        isAutoScrolling.value = false;
      }
    }

    Future<void> scrollToOldestUnread() async {
      final targetIndex = reversedIndexOf(oldestUnreadMessageId.value);
      if (targetIndex == null ||
          !itemScrollController.isAttached ||
          isAutoScrolling.value) {
        return;
      }
      isUnreadNavigationDismissed.value = true;
      followsLatest.value = false;
      hasUserScrolled.value = false;
      isAtLatest.value = false;
      isAutoScrolling.value = true;
      try {
        await itemScrollController.scrollTo(
          index: targetIndex,
          alignment: 0.35,
          duration: const Duration(milliseconds: 220),
          curve: Curves.easeOutCubic,
        );
      } finally {
        isAutoScrolling.value = false;
      }
    }

    bool latestIsAtBoundary() {
      // In this reversed list, item 0's leading edge is the visible bottom
      // boundary above the composer. Being merely visible is not enough: a
      // user who has pulled a tall newest row away from that boundary must not
      // snap back on live updates.
      final boundary = latestAlignment();
      return itemPositionsListener.itemPositions.value.any(
        (position) =>
            position.index == 0 &&
            (position.itemLeadingEdge - boundary).abs() < 0.01,
      );
    }

    void realignLatestAfterLayoutChange() {
      if (latestRealignmentQueued.value ||
          !followsLatest.value ||
          hasUserScrolled.value) {
        return;
      }
      latestRealignmentQueued.value = true;
      WidgetsBinding.instance.addPostFrameCallback((_) {
        latestRealignmentQueued.value = false;
        if (!context.mounted ||
            !itemScrollController.isAttached ||
            !followsLatest.value ||
            hasUserScrolled.value ||
            latestIsAtBoundary()) {
          return;
        }
        // A dock or keyboard resize is a layout correction, not a navigation
        // action. Keeping it instant avoids restarting a smooth scroll for
        // every position report while the viewport settles.
        itemScrollController.jumpTo(index: 0);
      });
    }

    useEffect(() {
      void onPositionsChanged() {
        final positions = itemPositionsListener.itemPositions.value;
        if (positions.isEmpty) return;
        final nextIsAtLatest = latestIsAtBoundary();
        if (showUnreadNavigation &&
            nextIsAtLatest &&
            detachedWhileUnreadShown.value) {
          isUnreadNavigationDismissed.value = true;
        }
        if (nextIsAtLatest) {
          if (!isAtLatest.value) isAtLatest.value = true;
        } else if (!followsLatest.value && isAtLatest.value) {
          isAtLatest.value = false;
        }

        final oldestVisible = positions
            .map((position) => position.index)
            .reduce((a, b) => a > b ? a : b);
        if (!hasUserScrolled.value ||
            oldestVisible < displayEntries.length - 3 ||
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

    // Composer size changes and keyboard metrics changes arrive in separate
    // layout passes. Preserve the latest-message anchor for both, but only
    // while the user has not deliberately left the tail.
    useEffect(() {
      realignLatestAfterLayoutChange();
      return null;
    }, [composerBottomInset]);

    useEffect(() {
      final observer = _ChannelLatestMetricsObserver(
        onMetricsChanged: realignLatestAfterLayoutChange,
      );
      WidgetsBinding.instance.addObserver(observer);
      return () => WidgetsBinding.instance.removeObserver(observer);
    }, [itemScrollController]);

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
        didJumpToInitialMessage.value = true;
        followsLatest.value = false;
        hasUserScrolled.value = false;
        isAtLatest.value = false;
        itemScrollController.jumpTo(index: targetIndex, alignment: 0.35);
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
            if (notification is UserScrollNotification &&
                notification.direction != ScrollDirection.idle) {
              hasUserScrolled.value = true;
              followsLatest.value = false;
              if (showUnreadNavigation) {
                detachedWhileUnreadShown.value = true;
              }
            } else if (notification is ScrollEndNotification &&
                hasUserScrolled.value) {
              WidgetsBinding.instance.addPostFrameCallback((_) {
                if (!context.mounted || !latestIsAtBoundary()) return;
                hasUserScrolled.value = false;
                followsLatest.value = true;
                if (!isAtLatest.value) isAtLatest.value = true;
              });
            }
            return false;
          },
          child: KeyboardDismissOnDrag(
            child: ScrollablePositionedList.builder(
              key: const ValueKey('channel-message-list'),
              itemScrollController: itemScrollController,
              itemPositionsListener: itemPositionsListener,
              reverse: true,
              padding: EdgeInsets.only(
                left: Grid.gutter,
                right: Grid.gutter,
                top: frostedAppBarHeight(
                  context,
                  titleContentHeight: appBarTitleContentHeight,
                ),
                bottom: composerBottomInset,
              ),
              itemCount: displayEntries.length + (isLoadingOlder.value ? 1 : 0),
              itemBuilder: (context, index) {
                // Loading indicator at the top (last index in reversed list).
                if (index >= displayEntries.length) {
                  return const Padding(
                    padding: EdgeInsets.symmetric(vertical: Grid.xs),
                    child: Center(
                      child: BuzzLoadingIndicator(
                        size: 24,
                        semanticLabel: 'Loading older messages',
                      ),
                    ),
                  );
                }

                // Reversed list: index 0 = newest (bottom of screen).
                final chronIdx = displayEntries.length - 1 - index;
                final entryGroup = displayEntries[chronIdx];
                final entry = entryGroup.first;
                final message = entry.message;

                // Day boundary check — applies to all messages including system.
                final prevEntry = chronIdx > 0
                    ? displayEntries[chronIdx - 1].last
                    : null;
                final prevMessage = prevEntry?.message;
                final showDayDivider =
                    prevMessage == null ||
                    !isSameDay(prevMessage.createdAt, message.createdAt);

                final showAuthor =
                    !message.isSystem &&
                    (message.hasAttachments ||
                        prevMessage == null ||
                        prevMessage.isSystem ||
                        showDayDivider ||
                        prevMessage.pubkey.toLowerCase() !=
                            message.pubkey.toLowerCase() ||
                        (message.createdAt - prevMessage.createdAt) > 300);

                return Padding(
                  key: ValueKey('channel-message-group-${message.id}'),
                  padding: EdgeInsets.only(bottom: index == 0 ? Grid.xs : 0),
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      if (showDayDivider)
                        DayDivider(label: formatDayHeading(message.createdAt)),
                      if (message.isSystem)
                        _SystemMessageRow(
                          message: message,
                          groupedMessages: entryGroup.length > 1
                              ? entryGroup
                                    .map((entry) => entry.message)
                                    .toList()
                              : null,
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
                  ),
                );
              },
            ),
          ),
        ),
        if (showUnreadNavigation)
          Positioned(
            left: 0,
            right: 0,
            top:
                frostedAppBarHeight(
                  context,
                  titleContentHeight: appBarTitleContentHeight,
                ) +
                Grid.xs,
            child: Center(
              child: IconButton.filled(
                key: const ValueKey('channel-jump-to-oldest-unread'),
                onPressed: scrollToOldestUnread,
                tooltip: 'Jump to oldest unread message',
                style: IconButton.styleFrom(
                  backgroundColor: context.colors.primaryContainer,
                  foregroundColor: context.colors.onPrimaryContainer,
                ),
                icon: const Icon(LucideIcons.chevronUp, size: 20),
              ),
            ),
          )
        else if (!isAtLatest.value)
          Positioned(
            left: 0,
            right: 0,
            bottom: composerBottomInset + Grid.xs,
            child: Center(
              child: _JumpToLatestButton(
                key: const ValueKey('channel-jump-to-latest'),
                onPressed: scrollToLatest,
              ),
            ),
          ),
      ],
    );
  }
}

class _JumpToLatestButton extends StatelessWidget {
  final VoidCallback onPressed;

  const _JumpToLatestButton({required this.onPressed, super.key});

  @override
  Widget build(BuildContext context) {
    final borderRadius = BorderRadius.circular(Radii.full);
    return Semantics(
      button: true,
      child: ClipRRect(
        borderRadius: borderRadius,
        child: BackdropFilter(
          filter: ImageFilter.blur(sigmaX: 20, sigmaY: 20),
          child: Container(
            key: const ValueKey('channel-jump-to-latest-surface'),
            decoration: BoxDecoration(
              color: context.colors.surface.withValues(alpha: 0.5),
              borderRadius: borderRadius,
              border: Border.all(
                color: Colors.black.withValues(alpha: 0.04),
                width: 1,
              ),
            ),
            child: Material(
              type: MaterialType.transparency,
              child: InkWell(
                onTap: onPressed,
                borderRadius: borderRadius,
                child: Padding(
                  padding: const EdgeInsets.symmetric(
                    horizontal: Grid.gutter,
                    vertical: Grid.xxs,
                  ),
                  child: Row(
                    mainAxisSize: MainAxisSize.min,
                    children: [
                      Icon(
                        LucideIcons.arrowDown,
                        size: 16,
                        color: context.colors.onSurface,
                      ),
                      const SizedBox(width: Grid.half),
                      Text(
                        'Latest',
                        style: context.textTheme.labelLarge?.copyWith(
                          color: context.colors.onSurface,
                        ),
                      ),
                    ],
                  ),
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }
}

class _ChannelLatestMetricsObserver with WidgetsBindingObserver {
  final VoidCallback onMetricsChanged;

  _ChannelLatestMetricsObserver({required this.onMetricsChanged});

  @override
  void didChangeMetrics() => onMetricsChanged();
}
