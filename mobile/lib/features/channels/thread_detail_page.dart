import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:scrollable_positioned_list/scrollable_positioned_list.dart';

import '../../shared/mentions/agent_identity_provider.dart';
import '../../shared/relay/relay.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/avatar_image.dart';
import '../../shared/widgets/frosted_app_bar.dart';
import '../../shared/widgets/frosted_scaffold.dart';
import '../../shared/widgets/keyboard_dismiss_on_drag.dart';
import '../../shared/widgets/message_author_meta.dart';
import '../profile/user_cache_provider.dart';
import '../profile/user_profile.dart';
import 'android_ime_lift.dart';
import 'channel_link_navigation.dart';
import 'channel_messages_provider.dart';
import 'channel_typing_provider.dart';
import 'channel_typing_indicator.dart';
import 'thread_replies_provider.dart';
import 'channels_provider.dart';
import 'compose_bar.dart';
import 'composer_dock_size_reporter.dart';
import 'date_formatters.dart';
import 'day_divider.dart';
import 'ime_metrics_settle_observer.dart';
import 'latest_message_button.dart';
import '../profile/user_profile_sheet.dart';
import 'initial_thread_tail_settle.dart';
import 'laid_out_viewport.dart';
import 'message_actions.dart';
import 'message_long_press_region.dart';
import 'message_content.dart';
import 'reaction_row.dart';
import '../../shared/read_state/read_state_format.dart';
import '../../shared/read_state/read_state_provider.dart';
import 'send_message_provider.dart';
import 'small_avatar.dart';
import 'timeline_message.dart';

part 'thread_detail_helpers.dart';
part 'thread_detail_page/nested_thread_summary_row.dart';
part 'thread_detail_page/tail_alignment.dart';
part 'thread_detail_page/thread_message.dart';

/// Full-screen thread detail page.
///
/// Shows the thread head message, direct replies, typing indicators scoped to
/// the thread, and a compose bar for replying.
class ThreadDetailPage extends HookConsumerWidget {
  final TimelineMessage threadHead;
  final List<TimelineMessage> allMessages;
  final String channelId;
  final String? currentPubkey;
  final bool isMember;
  final bool isArchived;
  final String? initialMessageId;

  const ThreadDetailPage({
    super.key,
    required this.threadHead,
    required this.allMessages,
    required this.channelId,
    required this.currentPubkey,
    required this.isMember,
    required this.isArchived,
    this.initialMessageId,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final appView = View.of(context);
    final composerDockHeight = useState(0.0);
    final settledImeBottomInset = useState(
      usesFixedAndroidImeViewport
          ? appView.viewInsets.bottom / appView.devicePixelRatio
          : 0.0,
    );
    final sendMessage = ref.read(sendMessageProvider);
    final queryRootId = threadHead.rootId ?? threadHead.id;
    final repliesState = ref.watch(
      threadRepliesWithLocalProvider(
        ThreadRepliesArgs(channelId: channelId, rootId: queryRootId),
      ),
    );
    final liveChannelEvents =
        ref.watch(channelMessagesProvider(channelId)).value ??
        const <NostrEvent>[];
    final replyMessages = repliesState.whenData((events) {
      return formatTimeline(
        mergeThreadEvents(events, liveChannelEvents),
        currentPubkey: currentPubkey,
      );
    });

    final fetchedReplies = replyMessages.value;
    final liveDeletionHidesHead = _isDeletedBy(
      liveChannelEvents,
      threadHead.id,
    );
    final allMsgs = fetchedReplies == null
        ? allMessages
        : [
            if (!liveDeletionHidesHead &&
                !fetchedReplies.any((message) => message.id == threadHead.id))
              threadHead,
            ...fetchedReplies,
          ];

    final childrenByParent = <String, List<TimelineMessage>>{};
    for (final msg in allMsgs) {
      final pid = msg.parentId;
      if (pid == null) continue;
      childrenByParent.putIfAbsent(pid, () => []).add(msg);
    }

    final replies = childrenByParent[threadHead.id] ?? const [];
    final itemScrollController = useMemoized(ItemScrollController.new);
    final itemPositionsListener = useMemoized(ItemPositionsListener.create);
    final listViewport = useMemoized(LaidOutViewport.new);
    useEffect(() => listViewport.dispose, [listViewport]);
    final didJumpToInitialMessage = useRef(false);
    final followsThreadTail = useRef(false);
    final userOptedOutOfTailFollow = useRef(false);
    final tailIntent = useMemoized(_ThreadTailIntent.new);
    final pendingTailAlignment = useRef<double?>(null);
    final isAtThreadTail = useState(true);
    final tailRealignmentQueued = useRef(false);
    final tailCorrectionInProgress = useRef(false);
    final initialTailSettle = useMemoized(InitialThreadTailSettle.new);
    final settledImeLift = usesFixedAndroidImeViewport
        ? (settledImeBottomInset.value -
                  MediaQuery.viewPaddingOf(context).bottom)
              .clamp(0.0, double.infinity)
              .toDouble()
        : 0.0;
    final timelineBottomInset =
        composerDockHeight.value +
        (isAtThreadTail.value || followsThreadTail.value ? settledImeLift : 0);
    final navigationBottomInset = composerDockHeight.value + settledImeLift;

    // Item 0 is the thread head; reply `i` lives at `i + 1`.
    const headIndex = 0;
    int indexForReply(int chronologicalIndex) => chronologicalIndex + 1;
    final tailAnchorIndex = replies.length + 1;

    double threadTailAlignment() => _threadTailAlignmentForViewport(
      fullHeight: MediaQuery.sizeOf(context).height,
      imeBottomInset: appView.viewInsets.bottom / appView.devicePixelRatio,
      usesFixedImeViewport: usesFixedAndroidImeViewport,
      bottomInset:
          Grid.xs +
          composerDockHeight.value +
          (followsThreadTail.value ? settledImeLift : 0),
    );

    bool threadTailIsVisible() {
      final targetAlignment = threadTailAlignment();
      return itemPositionsListener.itemPositions.value.any(
        (position) =>
            position.index == tailAnchorIndex &&
            position.itemLeadingEdge <= targetAlignment + 0.01,
      );
    }

    void correctThreadTailInstantly() {
      if (!itemScrollController.isAttached) return;
      tailCorrectionInProgress.value = true;
      isAtThreadTail.value = true;
      itemScrollController.jumpTo(
        index: tailAnchorIndex,
        alignment: threadTailAlignment(),
      );
      WidgetsBinding.instance.addPostFrameCallback((_) {
        tailCorrectionInProgress.value = false;
        if (context.mounted && followsThreadTail.value) {
          isAtThreadTail.value = true;
        }
      });
    }

    void followThreadTailFromComposer() {
      userOptedOutOfTailFollow.value = false;
      followsThreadTail.value = true;
      isAtThreadTail.value = true;
      tailIntent.schedule(
        allowed: true,
        revalidate: () =>
            context.mounted &&
            itemScrollController.isAttached &&
            !tailIntent.isDragging &&
            !userOptedOutOfTailFollow.value,
        action: correctThreadTailInstantly,
      );
    }

    useEffect(() {
      void onPositionsChanged() {
        final tailIsVisible = threadTailIsVisible();
        if (!userOptedOutOfTailFollow.value && tailIsVisible) {
          followsThreadTail.value = true;
        }
        if (tailCorrectionInProgress.value) return;
        if (isAtThreadTail.value != tailIsVisible) {
          isAtThreadTail.value = tailIsVisible;
        }
      }

      itemPositionsListener.itemPositions.addListener(onPositionsChanged);
      return () => itemPositionsListener.itemPositions.removeListener(
        onPositionsChanged,
      );
    }, [itemPositionsListener, replies.length]);

    void scrollToThreadLatest() {
      if (!itemScrollController.isAttached) return;
      userOptedOutOfTailFollow.value = false;
      followsThreadTail.value = true;
      // This state change rebuilds Android's keyboard-aware trailing padding
      // before the animated scroll targets the stable tail anchor.
      isAtThreadTail.value = true;
      tailIntent.schedule(
        allowed: true,
        revalidate: () =>
            context.mounted &&
            itemScrollController.isAttached &&
            !tailIntent.isDragging &&
            !userOptedOutOfTailFollow.value,
        action: () {
          itemScrollController.scrollTo(
            index: tailAnchorIndex,
            alignment: threadTailAlignment(),
            duration: const Duration(milliseconds: 220),
            curve: Curves.easeOutCubic,
          );
        },
      );
    }

    useEffect(() {
      final messageId = initialMessageId;
      if (messageId == null || fetchedReplies == null) return null;
      final chronologicalIndex = replies.indexWhere(
        (reply) => reply.id == messageId,
      );
      final targetIndex = messageId == threadHead.id
          ? headIndex
          : chronologicalIndex < 0
          ? null
          : indexForReply(chronologicalIndex);
      if (targetIndex == null || didJumpToInitialMessage.value) return null;
      didJumpToInitialMessage.value = true;
      tailIntent.schedule(
        allowed: true,
        revalidate: () =>
            context.mounted &&
            itemScrollController.isAttached &&
            !tailIntent.isDragging,
        action: () {
          tailIntent.detach();
          initialTailSettle.abandon();
          userOptedOutOfTailFollow.value = true;
          followsThreadTail.value = false;
          isAtThreadTail.value = false;
          pendingTailAlignment.value = null;
          itemScrollController.jumpTo(index: targetIndex, alignment: 0.35);
        },
      );
      return null;
    }, [initialMessageId, fetchedReplies, replies.length]);

    final hasFetchedReplies = fetchedReplies != null;
    final previousReplyCount = useRef(replies.length);
    final viewportHeight = useListenable(listViewport.height).value;
    final previousViewportHeight = useRef(viewportHeight);
    final topOverlayFraction = frostedAppBarHeight(context) / viewportHeight;
    final settleGeometry = (composerDockHeight.value, viewportHeight);
    bool currentIntentAllowsTailMutation({bool allowIdleDetached = false}) {
      if (tailIntent.isDragging) return false;
      if (allowIdleDetached) return true;
      return !userOptedOutOfTailFollow.value &&
          (followsThreadTail.value || threadTailIsVisible());
    }

    void queueTailRealignment({
      bool allowIdleDetached = false,
      bool restoreFollow = false,
      bool animate = false,
    }) {
      if (!initialTailSettle.isComplete ||
          viewportHeight <= 0 ||
          !currentIntentAllowsTailMutation(
            allowIdleDetached: allowIdleDetached,
          )) {
        return;
      }
      if (!allowIdleDetached) followsThreadTail.value = true;
      isAtThreadTail.value = true;
      tailIntent.schedule(
        allowed: true,
        revalidate: () =>
            context.mounted &&
            itemScrollController.isAttached &&
            currentIntentAllowsTailMutation(
              allowIdleDetached: allowIdleDetached,
            ),
        action: () {
          if (restoreFollow) {
            userOptedOutOfTailFollow.value = false;
            followsThreadTail.value = true;
          }
          if (animate) {
            itemScrollController.scrollTo(
              index: tailAnchorIndex,
              alignment: threadTailAlignment(),
              duration: const Duration(milliseconds: 220),
              curve: Curves.easeOutCubic,
            );
          } else {
            itemScrollController.jumpTo(
              index: tailAnchorIndex,
              alignment: threadTailAlignment(),
            );
          }
        },
      );
    }

    useEffect(() {
      if (!hasFetchedReplies || viewportHeight <= 0) return null;
      if (isMember && !isArchived && composerDockHeight.value <= 0) {
        return null;
      }
      if (!initialTailSettle.isComplete) {
        previousReplyCount.value = replies.length;
        previousViewportHeight.value = viewportHeight;
        initialTailSettle.schedule(
          context: context,
          controller: itemScrollController,
          positionsListener: itemPositionsListener,
          targetIndex: initialMessageId == null && replies.isNotEmpty
              ? indexForReply(replies.length - 1)
              : null,
          hiddenTopFraction: topOverlayFraction,
          hiddenBottomFraction: timelineBottomInset / viewportHeight,
        );
        return null;
      }
      final previous = previousReplyCount.value;
      previousReplyCount.value = replies.length;
      final viewportChanged =
          (viewportHeight - previousViewportHeight.value).abs() >= 0.5;
      previousViewportHeight.value = viewportHeight;
      if (replies.length <= previous) {
        // Preserve a short thread's valid top anchor when resize leaves its
        // tail inside the newly measured usable viewport. Long/clipped tails
        // still follow through the shared intent-serialized correction path.
        if (viewportChanged && !threadTailIsVisible()) {
          queueTailRealignment(animate: false);
        }
        return null;
      }
      final positions = itemPositionsListener.itemPositions.value;
      // Positions still describe the list as it was *before* these replies, so
      // compare against the old tail. Measuring against the new one only reads
      // as "at the tail" when exactly one reply arrived.
      final previousTailAnchorIndex = previous + 1;
      final wasAtTail = positions.any(
        (position) => position.index == previousTailAnchorIndex,
      );
      final localPubkey = currentPubkey?.toLowerCase();
      final hasNewLocalReply =
          localPubkey != null &&
          replies
              .skip(previous)
              .any((reply) => reply.pubkey.toLowerCase() == localPubkey);
      if (tailIntent.isDragging) return null;
      if (!hasNewLocalReply && (userOptedOutOfTailFollow.value || !wasAtTail)) {
        return null;
      }
      queueTailRealignment(
        allowIdleDetached: hasNewLocalReply,
        restoreFollow: hasNewLocalReply,
      );
      return null;
    }, [hasFetchedReplies, replies.length, settleGeometry]);
    final readState = ref.watch(readStateProvider);
    final visibleReplyReadKey = replies
        .map((reply) => '${reply.id}:${reply.createdAt}')
        .join(',');

    useEffect(() {
      if (!readState.isReady || replies.isEmpty) return null;
      WidgetsBinding.instance.addPostFrameCallback((_) {
        for (final reply in replies) {
          ref
              .read(readStateProvider.notifier)
              .markContextRead(msgContextKey(reply.id), reply.createdAt);
        }
      });
      return null;
    }, [threadHead.id, readState.isReady, visibleReplyReadKey]);

    final allTyping = ref.watch(channelTypingProvider(channelId));
    final threadTyping = allTyping
        .where((e) => e.threadHeadId == threadHead.id)
        .where(
          (e) =>
              currentPubkey == null ||
              e.pubkey.toLowerCase() != currentPubkey?.toLowerCase(),
        )
        .toList();

    final liveHead =
        allMsgs.where((m) => m.id == threadHead.id).firstOrNull ?? threadHead;

    final effectiveRootId = threadHead.rootId ?? threadHead.id;

    // Composer size changes and keyboard metrics changes are independent:
    // the dock grows first, then the Scaffold's viewport shrinks once the
    // keyboard appears. Re-align after that latter layout pass too, but only
    // while the user was already following the thread tail.
    void realignThreadTailAfterMetricsChange() {
      listViewport.reportAfterLayout();
      final shouldFollowTail =
          !userOptedOutOfTailFollow.value &&
          (followsThreadTail.value || threadTailIsVisible());
      if (!shouldFollowTail || tailRealignmentQueued.value) return;
      followsThreadTail.value = true;
      isAtThreadTail.value = true;
      tailRealignmentQueued.value = true;
      WidgetsBinding.instance.addPostFrameCallback((_) {
        tailRealignmentQueued.value = false;
        if (!context.mounted ||
            !itemScrollController.isAttached ||
            userOptedOutOfTailFollow.value ||
            !followsThreadTail.value ||
            tailIntent.isDragging) {
          return;
        }
        queueTailRealignment();
      });
    }

    void updateComposerDockHeight(double height) {
      listViewport.reportAfterLayout();
      final previousHeight = composerDockHeight.value;
      final heightDelta = height - previousHeight;
      if (heightDelta.abs() < 0.5) return;

      final shouldFollowTail =
          !userOptedOutOfTailFollow.value &&
          (followsThreadTail.value || threadTailIsVisible());
      if (shouldFollowTail) {
        followsThreadTail.value = true;
        isAtThreadTail.value = true;
      }
      composerDockHeight.value = height;
      if (heightDelta <= 0 ||
          !shouldFollowTail ||
          !viewportHeight.isFinite ||
          viewportHeight <= 0 ||
          !initialTailSettle.isComplete) {
        pendingTailAlignment.value = null;
        return;
      }
      final anchorPosition = itemPositionsListener.itemPositions.value
          .where((position) => position.index == tailAnchorIndex)
          .firstOrNull;
      if (anchorPosition == null) return;
      final targetAlignment =
          (pendingTailAlignment.value ?? anchorPosition.itemLeadingEdge) -
          (heightDelta / viewportHeight);
      pendingTailAlignment.value = targetAlignment;

      tailIntent.schedule(
        allowed: true,
        revalidate: () =>
            context.mounted &&
            itemScrollController.isAttached &&
            currentIntentAllowsTailMutation(),
        action: () => itemScrollController.jumpTo(
          index: tailAnchorIndex,
          alignment: targetAlignment,
        ),
      );
    }

    useEffect(() {
      final observer = ImeMetricsSettleObserver(
        onMetricsSettled: () {
          if (!usesFixedAndroidImeViewport) {
            realignThreadTailAfterMetricsChange();
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
    }, [appView, itemScrollController, replies.length]);

    useEffect(() {
      if (usesFixedAndroidImeViewport) {
        realignThreadTailAfterMetricsChange();
      }
      return null;
    }, [settledImeBottomInset.value]);

    final channelsAsync = ref.watch(channelsProvider);
    final channel = channelsAsync.value
        ?.where((candidate) => candidate.id == channelId)
        .firstOrNull;
    final channelNamesMap = <String, String>{};
    channelsAsync.whenData((channels) {
      for (final ch in channels) {
        channelNamesMap[ch.name.toLowerCase()] = ch.id;
      }
    });

    return FrostedScaffold(
      resizeToAvoidBottomInset: !usesFixedAndroidImeViewport,
      appBar: const FrostedAppBar(
        title: Text('Thread'),
        titleStyle: channelTitleTextStyle,
      ),
      body: Stack(
        fit: StackFit.expand,
        children: [
          Column(
            children: [
              Expanded(
                child: LaidOutViewportReporter(
                  viewport: listViewport,
                  child: KeyboardDismissOnDrag(
                    onUserScrollStart: () {
                      initialTailSettle.abandon();
                      tailIntent.beginDrag();
                      userOptedOutOfTailFollow.value = true;
                      followsThreadTail.value = false;
                      isAtThreadTail.value = false;
                      pendingTailAlignment.value = null;
                    },
                    onUserScrollEnd: () {
                      tailIntent.endDrag();
                      tailIntent.schedule(
                        allowed: userOptedOutOfTailFollow.value,
                        revalidate: () =>
                            context.mounted &&
                            itemScrollController.isAttached &&
                            !tailIntent.isDragging &&
                            userOptedOutOfTailFollow.value,
                        action: () {
                          _resumeThreadTailFollow(
                            isVisible: threadTailIsVisible,
                            userOptedOut: userOptedOutOfTailFollow,
                            followsTail: followsThreadTail,
                          );
                          if (threadTailIsVisible()) {
                            isAtThreadTail.value = true;
                          }
                        },
                      );
                    },
                    child: ScrollablePositionedList.builder(
                      key: const ValueKey('thread-message-list'),
                      itemScrollController: itemScrollController,
                      itemPositionsListener: itemPositionsListener,
                      // Top-anchored, head first, replies flowing down — matching
                      // desktop's thread panel. The old reversed list bottom-anchored
                      // the content, which jammed the head against the composer
                      // whenever a thread had only a handful of replies.
                      padding: EdgeInsets.only(
                        left: Grid.gutter,
                        right: Grid.gutter,
                        top: frostedAppBarHeight(context),
                        bottom: Grid.xs + timelineBottomInset,
                      ),
                      // Head + replies + a stable zero-content tail target. The
                      // anchor lets Latest align the end directly rather than
                      // asking the final reply's leading edge to overshoot the
                      // viewport and rebound against the scroll extent.
                      itemCount: replies.length + 2,
                      itemBuilder: (context, index) {
                        if (index == tailAnchorIndex) {
                          return const SizedBox(
                            key: ValueKey('thread-tail-anchor'),
                            height: 1,
                          );
                        }
                        if (index == headIndex) {
                          if (liveDeletionHidesHead) {
                            return const Padding(
                              key: ValueKey('thread-message-deleted'),
                              padding: EdgeInsets.only(bottom: Grid.xs),
                              child: Text('This message was deleted'),
                            );
                          }
                          return Padding(
                            key: ValueKey(
                              'thread-message-group-${liveHead.id}',
                            ),
                            padding: const EdgeInsets.only(bottom: Grid.xs),
                            child: Column(
                              crossAxisAlignment: CrossAxisAlignment.start,
                              children: [
                                DayDivider(
                                  label: formatDayHeading(liveHead.createdAt),
                                ),
                                _ThreadMessage(
                                  message: liveHead,
                                  channelNames: channelNamesMap,
                                  channelId: channelId,
                                  currentPubkey: currentPubkey,
                                  showAuthor: true,
                                  isHighlighted:
                                      liveHead.id == initialMessageId,
                                  allMessages: allMsgs,
                                  isMember: isMember,
                                  isArchived: isArchived,
                                  isThreadHead: true,
                                ),
                                Padding(
                                  padding: const EdgeInsets.symmetric(
                                    vertical: Grid.xxs,
                                  ),
                                  child: Row(
                                    children: [
                                      Text(
                                        '${replies.length} ${replies.length == 1 ? 'reply' : 'replies'}',
                                        style: context.textTheme.labelMedium
                                            ?.copyWith(
                                              color: context
                                                  .colors
                                                  .onSurfaceVariant,
                                              fontWeight: FontWeight.w600,
                                            ),
                                      ),
                                      const SizedBox(width: Grid.xxs),
                                      Expanded(
                                        child: Divider(
                                          color: context.colors.outlineVariant,
                                        ),
                                      ),
                                    ],
                                  ),
                                ),
                              ],
                            ),
                          );
                        }

                        // Chronological list: index 1 = oldest reply.
                        final chronIdx = index - 1;
                        final reply = replies[chronIdx];
                        final prevReply = chronIdx > 0
                            ? replies[chronIdx - 1]
                            : null;
                        final previousMessage = prevReply ?? liveHead;
                        final showDayDivider = !isSameDay(
                          previousMessage.createdAt,
                          reply.createdAt,
                        );
                        final showAuthor =
                            prevReply == null ||
                            showDayDivider ||
                            prevReply.pubkey.toLowerCase() !=
                                reply.pubkey.toLowerCase() ||
                            (reply.createdAt - prevReply.createdAt) > 300;

                        // Check if this reply itself has children (nested thread).
                        final nestedChildren = childrenByParent[reply.id];
                        final nestedSummary =
                            nestedChildren != null && nestedChildren.isNotEmpty
                            ? _buildNestedSummary(reply.id, nestedChildren)
                            : null;

                        return Padding(
                          key: ValueKey('thread-message-group-${reply.id}'),
                          // Tail spacing comes from the list's own bottom padding now
                          // that the list runs top-down; the reversed list used to
                          // need it here because item 0 sat against the composer.
                          padding: EdgeInsets.zero,
                          child: Column(
                            crossAxisAlignment: CrossAxisAlignment.start,
                            children: [
                              if (showDayDivider)
                                DayDivider(
                                  label: formatDayHeading(reply.createdAt),
                                ),
                              _ThreadMessage(
                                message: reply,
                                channelNames: channelNamesMap,
                                channelId: channelId,
                                currentPubkey: currentPubkey,
                                showAuthor: showAuthor,
                                isHighlighted: reply.id == initialMessageId,
                                allMessages: allMsgs,
                                isMember: isMember,
                                isArchived: isArchived,
                              ),
                              if (nestedSummary != null)
                                _NestedThreadSummaryRow(
                                  summary: nestedSummary,
                                  replyMessage: reply,
                                  allMessages: allMsgs,
                                  channelId: channelId,
                                  currentPubkey: currentPubkey,
                                  isMember: isMember,
                                  isArchived: isArchived,
                                ),
                            ],
                          ),
                        );
                      },
                    ),
                  ),
                ),
              ),
              if (!isMember || isArchived)
                _ThreadTypingIndicator(entries: threadTyping, animated: false),
            ],
          ),
          if (isMember && !isArchived)
            AndroidImeLift(
              child: Align(
                alignment: Alignment.bottomCenter,
                child: ComposerDockSizeReporter(
                  key: const ValueKey('thread-composer-dock'),
                  onHeightChanged: updateComposerDockHeight,
                  child: Column(
                    mainAxisSize: MainAxisSize.min,
                    children: [
                      _ThreadTypingIndicator(entries: threadTyping),
                      ComposeBar(
                        channelId: channelId,
                        hintText: 'Reply in thread\u2026',
                        threadHeadId: threadHead.id,
                        rootId: effectiveRootId,
                        onFocusRequested: followThreadTailFromComposer,
                        onSend:
                            (
                              content,
                              mentionPubkeys, {
                              mediaTags = const <List<String>>[],
                            }) => sendMessage.call(
                              channelId: channelId,
                              content: content,
                              mentionPubkeys: mentionPubkeys,
                              channel: channel,
                              parentEventId: threadHead.id,
                              rootEventId: effectiveRootId,
                              mediaTags: mediaTags,
                            ),
                      ),
                    ],
                  ),
                ),
              ),
            ),
          if (!isAtThreadTail.value)
            Positioned(
              left: 0,
              right: 0,
              bottom: navigationBottomInset + Grid.xs,
              child: Center(
                child: LatestMessageButton(
                  key: const ValueKey('thread-jump-to-latest'),
                  surfaceKey: const ValueKey('thread-jump-to-latest-surface'),
                  onPressed: scrollToThreadLatest,
                ),
              ),
            ),
        ],
      ),
    );
  }
}
