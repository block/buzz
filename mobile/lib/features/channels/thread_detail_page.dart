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
import 'jump_to_latest_button.dart';
import '../profile/user_profile_sheet.dart';
import 'initial_thread_tail_settle.dart';
import 'laid_out_viewport.dart';
import 'message_actions.dart';
import 'message_long_press_region.dart';
import 'message_content.dart';
import 'reaction_row.dart';
import '../../shared/read_state/message_read_state.dart';
import '../../shared/read_state/read_state_format.dart';
import '../../shared/read_state/read_state_provider.dart';
import 'send_message_provider.dart';
import 'small_avatar.dart';
import 'thread_unread_marker.dart';
import 'timeline_message.dart';
import 'unread_divider.dart';

part 'thread_detail_helpers.dart';
part 'thread_detail_page/nested_thread_summary.dart';
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
    final composerDockHeight = useState(0.0);
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
    const headIndex = 0;
    int indexForReply(int chronologicalIndex) => chronologicalIndex + 1;

    double threadTailTrailingBoundary() => _threadTailTrailingBoundary(
      hasComposerDock: isMember && !isArchived,
      viewportHeight: listViewport.height.value,
      dockHeight: composerDockHeight.value,
    );

    bool threadTailIsVisible() {
      final lastIndex = _threadTailIndex(replies.length);
      final trailingBoundary = threadTailTrailingBoundary();
      return itemPositionsListener.itemPositions.value.any(
        (position) =>
            position.index == lastIndex &&
            position.itemTrailingEdge <= trailingBoundary,
      );
    }

    // Pure-geometry "is the newest reply on screen" signal. Distinct from
    // [followsThreadTail], which means "auto-scroll is armed" and is set true
    // from the short route snapshot — so it never reports scrolled-away on a
    // long thread and cannot drive the jump-to-latest pill.
    final isAtThreadTail = useState(true);

    useEffect(() {
      void onPositionsChanged() {
        // An empty position set means the list has not laid out yet; treating
        // that as "scrolled away" would flash the pill on open.
        if (itemPositionsListener.itemPositions.value.isEmpty) return;
        final atTail = threadTailIsVisible();
        if (!userOptedOutOfTailFollow.value && atTail) {
          followsThreadTail.value = true;
        }
        isAtThreadTail.value = atTail;
      }

      itemPositionsListener.itemPositions.addListener(onPositionsChanged);
      return () => itemPositionsListener.itemPositions.removeListener(
        onPositionsChanged,
      );
    }, [itemPositionsListener, replies.length]);

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
          followsThreadTail.value = false;
          pendingTailAlignment.value = null;
          itemScrollController.jumpTo(index: targetIndex, alignment: 0.35);
        },
      );
      return null;
    }, [initialMessageId, fetchedReplies, replies.length]);

    final readState = ref.watch(readStateProvider);

    // Freeze each reply's read timestamp the first time it is rendered.
    // This must happen during build: the effect below marks every loaded
    // reply read in a post-frame callback, and markers are monotonic, so a
    // snapshot taken any later would already be the post-mark value and the
    // divider would collapse the instant the thread opened. `putIfAbsent`
    // makes the first capture win; a captured `null` (never read) is a real
    // answer, so reads below use `containsKey` rather than `??`.
    final openReadSnapshot = useRef(<String, int?>{});
    if (readState.isReady) {
      for (final reply in replies) {
        openReadSnapshot.value.putIfAbsent(
          reply.id,
          // Must be the folded resolver, not a bare msg: lookup. Opening a
          // channel writes only the channel marker, so on a thread's first
          // open every reply's own msg: marker is still null — a bare lookup
          // would call the whole thread unread and resume at reply #1, while
          // the long-press menu (isMessageUnread, same resolver) called it
          // read.
          () => effectiveMessageReadAt(
            readState,
            channelId: channelId,
            messageId: reply.id,
            threadRootId: queryRootId,
          ),
        );
      }
    }

    // Oldest reply the user has not seen, from the frozen snapshot.
    final firstUnreadReplyId = readState.isReady
        ? firstUnreadThreadReplyId(
            replies: replies,
            openReadSnapshot: openReadSnapshot.value,
            isForcedUnread: (messageId) =>
                readState.isForcedUnread(msgContextKey(messageId)),
            currentPubkey: currentPubkey,
          )
        : null;
    final firstUnreadReplyIndex = firstUnreadReplyId == null
        ? -1
        : replies.indexWhere((reply) => reply.id == firstUnreadReplyId);

    final hasFetchedReplies = fetchedReplies != null;
    final initialTailSettle = useMemoized(InitialThreadTailSettle.new);
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
      bool animate = true,
    }) {
      if (!initialTailSettle.isComplete ||
          viewportHeight <= 0 ||
          !currentIntentAllowsTailMutation(
            allowIdleDetached: allowIdleDetached,
          )) {
        return;
      }
      if (!allowIdleDetached) followsThreadTail.value = true;
      tailIntent.schedule(
        allowed: true,
        revalidate: () =>
            context.mounted &&
            itemScrollController.isAttached &&
            currentIntentAllowsTailMutation(
              allowIdleDetached: allowIdleDetached,
            ),
        action: () {
          final lastIndex = _threadTailIndex(replies.length);
          if (restoreFollow) {
            userOptedOutOfTailFollow.value = false;
            followsThreadTail.value = true;
          }
          if (animate) {
            itemScrollController.scrollTo(
              index: lastIndex,
              alignment: topOverlayFraction,
              duration: const Duration(milliseconds: 220),
              curve: Curves.easeOutCubic,
            );
          } else {
            itemScrollController.jumpTo(
              index: lastIndex,
              alignment: topOverlayFraction,
            );
          }
        },
      );
    }

    // Tapping the pill is an explicit request for the tail, so it retires any
    // pending initial settle and clears the opt-out a manual scroll set.
    Future<void> scrollToThreadLatest() async {
      if (!itemScrollController.isAttached || replies.isEmpty) return;
      final lastIndex = _threadTailIndex(replies.length);
      initialTailSettle.abandon();
      tailIntent.detach();
      userOptedOutOfTailFollow.value = false;
      followsThreadTail.value = true;
      pendingTailAlignment.value = null;
      await itemScrollController.scrollTo(
        index: lastIndex,
        alignment: topOverlayFraction,
        duration: const Duration(milliseconds: 220),
        curve: Curves.easeOutCubic,
      );
      if (!context.mounted || !itemScrollController.isAttached) return;
      // scrollTo pins the item's LEADING edge below the app bar. For a reply
      // taller than the usable viewport the trailing edge is then still off
      // screen — precisely the state threadTailIsVisible() reports as "not at
      // the tail" — so the pill would stay up and a second tap would do
      // nothing. Nudge by the measured overflow so the trailing edge lands on
      // the boundary that predicate uses.
      final trailingBoundary = threadTailTrailingBoundary();
      if (!trailingBoundary.isFinite) return;
      final position = itemPositionsListener.itemPositions.value
          .where((position) => position.index == lastIndex)
          .firstOrNull;
      if (position == null || position.itemTrailingEdge <= trailingBoundary) {
        return;
      }
      itemScrollController.jumpTo(
        index: lastIndex,
        alignment:
            position.itemLeadingEdge -
            (position.itemTrailingEdge - trailingBoundary),
      );
    }

    useEffect(() {
      if (!hasFetchedReplies || viewportHeight <= 0) return null;
      if (isMember && !isArchived && composerDockHeight.value <= 0) {
        return null;
      }
      if (!initialTailSettle.isComplete) {
        // Read markers decide where an ordinary open lands, so hold the settle
        // until they load rather than committing to the tail and jumping
        // again a moment later. A deep link names its own target, so it must
        // not wait — otherwise a marker fetch that never resolves would leave
        // the settle incomplete and keep tail realignment disabled for good.
        // The first user scroll abandons the settle either way.
        if (initialMessageId == null && !readState.isReady) return null;
        previousReplyCount.value = replies.length;
        previousViewportHeight.value = viewportHeight;
        initialTailSettle.schedule(
          context: context,
          controller: itemScrollController,
          positionsListener: itemPositionsListener,
          // Resume at the oldest unread reply, falling back to the tail once
          // the thread is fully read. A deep link names its own target and
          // always wins.
          targetIndex: initialMessageId == null && replies.isNotEmpty
              ? indexForReply(
                  firstUnreadReplyIndex >= 0
                      ? firstUnreadReplyIndex
                      : replies.length - 1,
                )
              : null,
          hiddenTopFraction: topOverlayFraction,
          hiddenBottomFraction: composerDockHeight.value / viewportHeight,
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
      final previousLastIndex = previous == 0
          ? headIndex
          : indexForReply(previous - 1);
      final wasAtTail = positions.any(
        (position) => position.index == previousLastIndex,
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
    }, [
      hasFetchedReplies,
      readState.isReady,
      replies.length,
      firstUnreadReplyId,
      settleGeometry,
    ]);
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

    void updateComposerDockHeight(double height) {
      listViewport.reportAfterLayout();
      final previousHeight = composerDockHeight.value;
      final heightDelta = height - previousHeight;
      if (heightDelta.abs() < 0.5) return;

      final shouldFollowTail =
          !userOptedOutOfTailFollow.value &&
          (followsThreadTail.value || threadTailIsVisible());
      if (shouldFollowTail) followsThreadTail.value = true;
      composerDockHeight.value = height;
      if (heightDelta <= 0 ||
          !shouldFollowTail ||
          !viewportHeight.isFinite ||
          viewportHeight <= 0 ||
          !initialTailSettle.isComplete) {
        pendingTailAlignment.value = null;
        return;
      }
      final lastIndex = _threadTailIndex(replies.length);
      final lastPosition = itemPositionsListener.itemPositions.value
          .where((position) => position.index == lastIndex)
          .firstOrNull;
      if (lastPosition == null) return;
      final targetAlignment =
          (pendingTailAlignment.value ?? lastPosition.itemLeadingEdge) -
          (heightDelta / viewportHeight);
      pendingTailAlignment.value = targetAlignment;

      tailIntent.schedule(
        allowed: true,
        revalidate: () =>
            context.mounted &&
            itemScrollController.isAttached &&
            currentIntentAllowsTailMutation(),
        action: () => itemScrollController.jumpTo(
          index: lastIndex,
          alignment: targetAlignment,
        ),
      );
    }

    void realignThreadTailAfterMetricsChange() {
      listViewport.reportAfterLayout();
      queueTailRealignment();
    }

    useEffect(() {
      final observer = _ThreadTailMetricsObserver(
        onMetricsChanged: realignThreadTailAfterMetricsChange,
      );
      WidgetsBinding.instance.addObserver(observer);
      return () => WidgetsBinding.instance.removeObserver(observer);
    }, [itemScrollController, replies.length]);

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
                        action: () => _resumeThreadTailFollow(
                          isVisible: threadTailIsVisible,
                          userOptedOut: userOptedOutOfTailFollow,
                          followsTail: followsThreadTail,
                        ),
                      );
                    },
                    child: ScrollablePositionedList.builder(
                      key: const ValueKey('thread-message-list'),
                      itemScrollController: itemScrollController,
                      itemPositionsListener: itemPositionsListener,
                      padding: EdgeInsets.only(
                        left: Grid.gutter,
                        right: Grid.gutter,
                        top: frostedAppBarHeight(context),
                        bottom: Grid.xs + composerDockHeight.value,
                      ),
                      itemCount: replies.length + 1, // +1 for thread head
                      itemBuilder: (context, index) {
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

                        final nestedChildren = childrenByParent[reply.id];
                        final nestedSummary =
                            nestedChildren != null && nestedChildren.isNotEmpty
                            ? _buildNestedSummary(reply.id, nestedChildren)
                            : null;

                        return Padding(
                          key: ValueKey('thread-message-group-${reply.id}'),
                          padding: EdgeInsets.zero,
                          child: Column(
                            crossAxisAlignment: CrossAxisAlignment.start,
                            children: [
                              if (showDayDivider)
                                DayDivider(
                                  label: formatDayHeading(reply.createdAt),
                                ),
                              if (reply.id == firstUnreadReplyId)
                                const UnreadDivider(
                                  key: ValueKey('thread-unread-divider'),
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
            Align(
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
          // A thread with no replies has no "latest" to jump to; the head's
          // own scroll position is not what this control means.
          if (!isAtThreadTail.value && replies.isNotEmpty)
            Positioned(
              left: 0,
              right: 0,
              bottom: composerDockHeight.value + Grid.xs,
              child: Center(
                child: JumpToLatestButton(
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
