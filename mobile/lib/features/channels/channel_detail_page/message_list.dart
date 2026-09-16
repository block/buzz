part of '../channel_detail_page.dart';

class _MessageList extends HookConsumerWidget {
  final List<MainTimelineEntry> entries;
  final List<TimelineMessage> allMessages;
  final String? initialMessageId;
  final String? initialThreadRootId;
  final InitialThreadRouteBehavior initialThreadRouteBehavior;
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
  final FocusNode? composerFocusNode;
  final VoidCallback? restoreComposerFocus;

  const _MessageList({
    required this.entries,
    required this.allMessages,
    required this.initialMessageId,
    required this.initialThreadRootId,
    required this.initialThreadRouteBehavior,
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
    this.composerFocusNode,
    this.restoreComposerFocus,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final appView = View.of(context);
    final localSendAnimations = ref.watch(
      localMessageSendAnimationProvider(channelId),
    );
    final displayEntries = groupMembershipTimelineEntries(entries);
    final itemScrollController = useMemoized(ItemScrollController.new);
    final itemPositionsListener = useMemoized(ItemPositionsListener.create);
    final stickyDateHeaderState = useValueNotifier(
      StickyDateHeaderState.hidden,
    );
    final stickyDayTimestamp = useValueNotifier<int?>(null);
    final timelineViewportHeight = useRef(MediaQuery.sizeOf(context).height);
    final isLoadingOlder = useState(false);
    final isAtLatest = useState(true);
    final settledImeBottomInset = useState(
      usesFixedAndroidImeViewport
          ? appView.viewInsets.bottom / appView.devicePixelRatio
          : 0.0,
    );
    final isJumpToLatestVisible = useState(false);
    final hasUserScrolled = useState(false);
    final distanceFromLatest = useRef(0.0);
    final hasUnseenLatestEntry = useRef(false);
    final followsLatest = useState(
      initialMessageId == null && initialThreadRootId == null,
    );
    final isAutoScrolling = useRef(false);
    final latestNavigationRequest = useState(0);
    final latestNavigationTargetId = useRef<String?>(null);
    final latestRealignmentQueued = useRef(false);
    final latestEntryId = entries.isEmpty ? null : entries.last.message.id;
    final previousLatestEntryId = useRef<String?>(null);
    final observedLocalSendIds = useRef(localSendAnimations.keys.toSet());
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
    final dayTimestampByReversedIndex = <int, int>{};
    final dayStartByReversedIndex = <int, int>{};
    final dayHeaderTimestampByReversedIndex = <int, int>{};
    final settledImeLift = usesFixedAndroidImeViewport
        ? (settledImeBottomInset.value -
                  MediaQuery.viewPaddingOf(context).bottom)
              .clamp(0.0, double.infinity)
              .toDouble()
        : settledImeBottomInset.value;
    final timelineBottomInset =
        composerBottomInset + (followsLatest.value ? settledImeLift : 0);
    final navigationBottomInset = composerBottomInset + settledImeLift;
    var currentDayTimestamp =
        displayEntries.firstOrNull?.first.message.createdAt;
    var currentDayStartIndex = displayEntries.isEmpty
        ? -1
        : displayEntries.length - 1;
    for (
      var chronologicalIndex = 0;
      chronologicalIndex < displayEntries.length;
      chronologicalIndex += 1
    ) {
      final message = displayEntries[chronologicalIndex].first.message;
      final previousMessage = chronologicalIndex > 0
          ? displayEntries[chronologicalIndex - 1].last.message
          : null;
      final startsDay =
          previousMessage == null ||
          !isSameDay(previousMessage.createdAt, message.createdAt);
      final reversedIndex = displayEntries.length - 1 - chronologicalIndex;
      if (startsDay) {
        currentDayTimestamp = message.createdAt;
        currentDayStartIndex = reversedIndex;
        dayHeaderTimestampByReversedIndex[reversedIndex] = message.createdAt;
      }
      final dayTimestamp = currentDayTimestamp;
      if (dayTimestamp != null) {
        dayTimestampByReversedIndex[reversedIndex] = dayTimestamp;
        dayStartByReversedIndex[reversedIndex] = currentDayStartIndex;
      }
    }

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
      final viewportHeight = timelineViewportHeight.value;
      return viewportHeight > 0
          ? (timelineBottomInset / viewportHeight).clamp(0.0, 1.0).toDouble()
          : 0.0;
    }

    void updateStickyDateHeader(Iterable<ItemPosition> rawPositions) {
      void setStickyDateHeader(
        StickyDateHeaderState state, {
        int? activeDayTimestamp,
      }) {
        stickyDateHeaderState.value = state;
        stickyDayTimestamp.value = activeDayTimestamp;
      }

      final viewportHeight = timelineViewportHeight.value;
      if (viewportHeight <= 0 || displayEntries.isEmpty) {
        setStickyDateHeader(StickyDateHeaderState.hidden);
        return;
      }

      final positions = rawPositions
          .where(
            (position) =>
                position.index < displayEntries.length &&
                position.itemLeadingEdge < 1 &&
                position.itemTrailingEdge > 0,
          )
          .toList();
      if (positions.isEmpty) {
        if (!isLoadingOlder.value) {
          setStickyDateHeader(StickyDateHeaderState.hidden);
        }
        return;
      }

      final stickyTop =
          frostedAppBarHeight(
            context,
            titleContentHeight: appBarTitleContentHeight,
          ) +
          Grid.twelve;
      double physicalTop(ItemPosition position) =>
          viewportHeight * (1 - position.itemTrailingEdge);
      double physicalBottom(ItemPosition position) =>
          viewportHeight * (1 - position.itemLeadingEdge);

      final positionAtStickyTop = positions
          .where(
            (position) =>
                physicalTop(position) <= stickyTop &&
                physicalBottom(position) > stickyTop,
          )
          .firstOrNull;
      if (positionAtStickyTop == null) {
        if (!isLoadingOlder.value) {
          setStickyDateHeader(StickyDateHeaderState.hidden);
        }
        return;
      }

      final activeDayTimestamp =
          dayTimestampByReversedIndex[positionAtStickyTop.index];
      final activeDayStartIndex =
          dayStartByReversedIndex[positionAtStickyTop.index];
      if (activeDayTimestamp == null || activeDayStartIndex == null) {
        setStickyDateHeader(StickyDateHeaderState.hidden);
        return;
      }

      final activeHeaderPosition = positions
          .where((position) => position.index == activeDayStartIndex)
          .firstOrNull;
      final oldestVisibleIndex = positions
          .map((position) => position.index)
          .reduce((a, b) => a > b ? a : b);
      final activeHeaderHasCrossed = activeHeaderPosition != null
          ? physicalTop(activeHeaderPosition) <= stickyTop
          : activeDayStartIndex > oldestVisibleIndex;
      if (!activeHeaderHasCrossed) {
        setStickyDateHeader(StickyDateHeaderState.hidden);
        return;
      }

      double? nextHeaderTop;
      for (final position in positions) {
        if (!dayHeaderTimestampByReversedIndex.containsKey(position.index) ||
            position.index >= activeDayStartIndex) {
          continue;
        }
        final top = physicalTop(position);
        if (top <= stickyTop ||
            (nextHeaderTop != null && top >= nextHeaderTop)) {
          continue;
        }
        nextHeaderTop = top;
      }

      final stickyHeaderHeight = StickyDateHeader.heightOf(context);
      final rawTranslateY = nextHeaderTop == null
          ? 0.0
          : min(0.0, nextHeaderTop - stickyTop - stickyHeaderHeight - 5);
      final translateY = rawTranslateY
          .clamp(-(stickyHeaderHeight + 5), 0.0)
          .toDouble();
      setStickyDateHeader(
        StickyDateHeaderState(
          label: formatDayHeading(activeDayTimestamp),
          translateY: (translateY * 2).round() / 2,
        ),
        activeDayTimestamp: activeDayTimestamp,
      );
    }

    Future<void> performLatestNavigation() async {
      if (!context.mounted || !itemScrollController.isAttached) {
        isAutoScrolling.value = false;
        return;
      }
      try {
        final targetIndex =
            reversedIndexOf(latestNavigationTargetId.value) ?? 0;
        await itemScrollController.scrollTo(
          index: targetIndex,
          alignment: latestAlignment(),
          duration: jumpToLatestScrollDuration,
          curve: jumpToLatestScrollCurve,
        );
        if (context.mounted && !hasUserScrolled.value) {
          isAtLatest.value = targetIndex == 0;
          isJumpToLatestVisible.value = false;
        }
      } finally {
        isAutoScrolling.value = false;
      }
    }

    void scrollToLatest({String? targetMessageId}) {
      if (!itemScrollController.isAttached || isAutoScrolling.value) return;
      isAutoScrolling.value = true;
      followsLatest.value = true;
      hasUserScrolled.value = false;
      hasUnseenLatestEntry.value = false;
      latestNavigationTargetId.value = targetMessageId;
      latestNavigationRequest.value += 1;
    }

    useEffect(() {
      if (latestNavigationRequest.value == 0) return null;
      var cancelled = false;
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (cancelled) return;
        unawaited(performLatestNavigation());
      });
      return () => cancelled = true;
    }, [latestNavigationRequest.value]);

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

    void updateJumpToLatestVisibility(
      Iterable<ItemPosition> positions, {
      double? viewportDimension,
    }) {
      final latestIsVisible = positions.any(
        (position) =>
            position.index == 0 &&
            position.itemLeadingEdge < 1 &&
            position.itemTrailingEdge > latestAlignment(),
      );
      final viewportHeight = viewportDimension ?? timelineViewportHeight.value;
      final visiblePageHeight = max(
        0.0,
        viewportHeight -
            frostedAppBarHeight(
              context,
              titleContentHeight: appBarTitleContentHeight,
            ) -
            composerBottomInset,
      );
      final hidesJumpToLatest = shouldHideChannelJumpToLatest(
        isAtLatest: latestIsAtBoundary(),
        followsLatest: followsLatest.value,
        userHasDetached: hasUserScrolled.value,
      );
      final shouldShow =
          !hidesJumpToLatest &&
          (hasUnseenLatestEntry.value ||
              !latestIsVisible ||
              distanceFromLatest.value > visiblePageHeight);
      if (isJumpToLatestVisible.value != shouldShow) {
        isJumpToLatestVisible.value = shouldShow;
      }
    }

    void realignLatestAfterLayoutChange() {
      if (latestRealignmentQueued.value ||
          isAutoScrolling.value ||
          !followsLatest.value ||
          hasUserScrolled.value) {
        return;
      }
      latestRealignmentQueued.value = true;
      WidgetsBinding.instance.addPostFrameCallback((_) {
        latestRealignmentQueued.value = false;
        if (!context.mounted ||
            !itemScrollController.isAttached ||
            isAutoScrolling.value ||
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

    useEffect(
      () {
        void onPositionsChanged() {
          final positions = itemPositionsListener.itemPositions.value;
          if (positions.isEmpty) return;
          updateStickyDateHeader(positions);
          updateJumpToLatestVisibility(positions);
          final nextIsAtLatest = latestIsAtBoundary();
          if (showUnreadNavigation &&
              nextIsAtLatest &&
              detachedWhileUnreadShown.value) {
            isUnreadNavigationDismissed.value = true;
          }
          if (nextIsAtLatest) {
            hasUnseenLatestEntry.value = false;
            if (!isAtLatest.value) isAtLatest.value = true;
            if (isJumpToLatestVisible.value) {
              isJumpToLatestVisible.value = false;
            }
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
          final notifier = ref.read(
            channelMessagesProvider(channelId).notifier,
          );
          if (notifier.reachedOldest) return;
          isLoadingOlder.value = true;
          notifier.fetchOlder().whenComplete(
            () => isLoadingOlder.value = false,
          );
        }

        var disposed = false;
        itemPositionsListener.itemPositions.addListener(onPositionsChanged);
        WidgetsBinding.instance.addPostFrameCallback((_) {
          if (!disposed && context.mounted) onPositionsChanged();
        });
        return () {
          disposed = true;
          itemPositionsListener.itemPositions.removeListener(
            onPositionsChanged,
          );
        };
      },
      [
        channelId,
        entries.length,
        itemPositionsListener,
        appBarTitleContentHeight,
        composerBottomInset,
      ],
    );

    useEffect(() {
      stickyDateHeaderState.value = StickyDateHeaderState.hidden;
      stickyDayTimestamp.value = null;
      return null;
    }, [channelId]);

    // Composer size changes and keyboard metrics changes arrive in separate
    // layout passes. Preserve the latest-message anchor for both, but only
    // while the user has not deliberately left the tail.
    useEffect(() {
      realignLatestAfterLayoutChange();
      return null;
    }, [timelineBottomInset]);

    useEffect(() {
      final observer = ImeMetricsSettleObserver(
        onMetricsSettled: () {
          if (!usesFixedAndroidImeViewport) {
            realignLatestAfterLayoutChange();
            return;
          }
          final nextInset =
              appView.viewInsets.bottom / appView.devicePixelRatio;
          if ((settledImeBottomInset.value - nextInset).abs() >= 0.5) {
            settledImeBottomInset.value = nextInset;
          }
        },
      );
      WidgetsBinding.instance.addObserver(observer);
      return () {
        WidgetsBinding.instance.removeObserver(observer);
        observer.dispose();
      };
    }, [appView, itemScrollController]);

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
        if (!context.mounted || ModalRoute.of(context)?.isCurrent != true) {
          return;
        }
        final route = MaterialPageRoute<void>(
          builder: (_) => ThreadDetailPage(
            threadHead: threadHead,
            allMessages: allMessages,
            channelId: channelId,
            currentPubkey: currentPubkey,
            isMember: isMember,
            isArchived: isArchived,
            initialMessageId: initialMessageId,
          ),
        );
        final navigator = Navigator.of(context);
        switch (initialThreadRouteBehavior) {
          case InitialThreadRouteBehavior.push:
            navigator.push(route);
          case InitialThreadRouteBehavior.replaceCurrentRoute:
            navigator.pushReplacement(route);
        }
      });
      return null;
    }, [initialThreadRootId, allMessages, initialThreadRouteBehavior]);

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
      final entryIds = entries.map((entry) => entry.message.id).toSet();
      final newlyInsertedLocalSendIds = localSendAnimations.keys
          .where(
            (eventId) =>
                !observedLocalSendIds.value.contains(eventId) &&
                entryIds.contains(eventId),
          )
          .toList();
      observedLocalSendIds.value
        ..removeWhere((eventId) => !localSendAnimations.containsKey(eventId))
        ..addAll(newlyInsertedLocalSendIds);
      final localSendId = newlyInsertedLocalSendIds.firstOrNull;
      final latestEntryChanged =
          previous != null &&
          latestEntryId != null &&
          previous != latestEntryId;
      if (!latestEntryChanged && localSendId == null) {
        return null;
      }
      if (localSendId != null) {
        followsLatest.value = true;
        hasUserScrolled.value = false;
      }
      if (!followsLatest.value || hasUserScrolled.value) {
        hasUnseenLatestEntry.value = true;
      }
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (!context.mounted) return;
        if (followsLatest.value && !hasUserScrolled.value) {
          scrollToLatest(targetMessageId: localSendId);
          return;
        }
        final positions = itemPositionsListener.itemPositions.value;
        if (positions.isNotEmpty) {
          if (latestIsAtBoundary()) {
            hasUnseenLatestEntry.value = false;
          }
          updateJumpToLatestVisibility(positions);
        }
      });
      return null;
    }, [latestEntryId, localSendAnimations]);

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
        NotificationListener<Notification>(
          onNotification: (notification) {
            if (notification is ScrollMetricsNotification &&
                notification.depth != 0) {
              return false;
            }
            if (notification is ScrollNotification && notification.depth != 0) {
              return false;
            }
            if (notification is ScrollMetricsNotification) {
              timelineViewportHeight.value =
                  notification.metrics.viewportDimension;
              return false;
            }
            if (notification is! ScrollNotification) return false;
            timelineViewportHeight.value =
                notification.metrics.viewportDimension;
            distanceFromLatest.value = max(
              0.0,
              notification.metrics.pixels -
                  notification.metrics.minScrollExtent,
            );
            updateJumpToLatestVisibility(
              itemPositionsListener.itemPositions.value,
              viewportDimension: notification.metrics.viewportDimension,
            );
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
                bottom: timelineBottomInset,
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

                return LocalMessageSendTransition(
                  key: ValueKey('channel-message-send-${message.id}'),
                  animate: isRecentLocalMessageSendAnimation(
                    localSendAnimations,
                    message.id,
                  ),
                  startOffsetFactor: showAuthor
                      ? localMessageSendTransitionAvatarStartOffset
                      : localMessageSendTransitionStartOffset,
                  child: Padding(
                    key: ValueKey('channel-message-group-${message.id}'),
                    padding: EdgeInsets.only(bottom: index == 0 ? Grid.xs : 0),
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        if (showDayDivider)
                          DayDivider(
                            label: formatDayHeading(message.createdAt),
                            dayTimestamp: message.createdAt,
                            stickyDayTimestamp: stickyDayTimestamp,
                          ),
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
                            allMessages: allMessages,
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
                            composerFocusNode: composerFocusNode,
                            restoreComposerFocus: restoreComposerFocus,
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
                  ),
                );
              },
            ),
          ),
        ),
        if (!showUnreadNavigation)
          Positioned(
            left: 0,
            right: 0,
            top:
                frostedAppBarHeight(
                  context,
                  titleContentHeight: appBarTitleContentHeight,
                ) +
                Grid.twelve,
            child: StickyDateHeader(
              key: const ValueKey('channel-sticky-date-header'),
              state: stickyDateHeaderState,
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
        else
          Positioned(
            left: 0,
            right: 0,
            bottom: navigationBottomInset + Grid.xs,
            child: Center(
              child: JumpToLatestSwitcher(
                id: 'channel',
                visible: isJumpToLatestVisible.value,
                onPressed: scrollToLatest,
              ),
            ),
          ),
      ],
    );
  }
}
