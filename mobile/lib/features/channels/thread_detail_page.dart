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
import 'message_actions.dart';
import 'message_long_press_region.dart';
import 'message_content.dart';
import 'reaction_row.dart';
import '../../shared/read_state/read_state_format.dart';
import '../../shared/read_state/read_state_provider.dart';
import 'send_message_provider.dart';
import 'small_avatar.dart';
import 'timeline_message.dart';
import 'unread_divider.dart';

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
    // Relay thread queries are keyed by the outermost root, even when this
    // page displays a nested branch. Query that root, then select this head's
    // direct children from the returned subtree below.
    final queryRootId = threadHead.rootId ?? threadHead.id;
    final repliesState = ref.watch(
      threadRepliesWithLocalProvider(
        ThreadRepliesArgs(channelId: channelId, rootId: queryRootId),
      ),
    );
    // The thread query is one-shot and asks only for content kinds, so a
    // reaction, edit, or deletion that lands while the thread is open never
    // reaches it — a new pill (and its burst) only showed up after leaving and
    // re-entering, which refetched. The channel socket already receives those
    // events, so union the two sources and format once.
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
            // Only fall back to the pushed-route snapshot when neither source
            // carries the head, and no live deletion has suppressed it. That
            // keeps a temporarily unavailable head visible without restoring
            // a head that was deleted while this page was open.
            if (!liveDeletionHidesHead &&
                !fetchedReplies.any((message) => message.id == threadHead.id))
              threadHead,
            ...fetchedReplies,
          ];

    // Index all messages by parentId so we can find direct children of any
    // message and compute thread summaries for nested threads.
    final childrenByParent = <String, List<TimelineMessage>>{};
    for (final msg in allMsgs) {
      final pid = msg.parentId;
      if (pid == null) continue;
      childrenByParent.putIfAbsent(pid, () => []).add(msg);
    }

    final replies = childrenByParent[threadHead.id] ?? const [];
    final itemScrollController = useMemoized(ItemScrollController.new);
    final itemPositionsListener = useMemoized(ItemPositionsListener.create);
    final didJumpToInitialMessage = useRef(false);
    final followsThreadTail = useRef(false);
    final pendingTailAlignment = useRef<double?>(null);
    final tailRealignmentQueued = useRef(false);

    // Item 0 is the thread head; reply `i` lives at `i + 1`.
    const headIndex = 0;
    int indexForReply(int chronologicalIndex) => chronologicalIndex + 1;

    bool threadTailIsVisible() {
      final lastIndex = replies.isEmpty
          ? headIndex
          : indexForReply(replies.length - 1);
      return itemPositionsListener.itemPositions.value.any(
        (position) =>
            position.index == lastIndex && position.itemTrailingEdge <= 1.001,
      );
    }

    // Pure-geometry "is the newest reply on screen" signal. Distinct from
    // [followsThreadTail], which means "auto-scroll is armed" and is set true
    // on the first frame from the short route snapshot — so it never reports
    // scrolled-away on a long thread and cannot drive the jump-to-latest pill.
    final isAtThreadTail = useState(true);

    useEffect(() {
      void onPositionsChanged() {
        // An empty position set means the list has not laid out yet; treating
        // that as "scrolled away" would flash the pill on open.
        if (itemPositionsListener.itemPositions.value.isEmpty) return;
        final atTail = threadTailIsVisible();
        if (atTail) followsThreadTail.value = true;
        isAtThreadTail.value = atTail;
      }

      itemPositionsListener.itemPositions.addListener(onPositionsChanged);
      return () => itemPositionsListener.itemPositions.removeListener(
        onPositionsChanged,
      );
    }, [itemPositionsListener, replies.length]);

    Future<void> scrollToThreadLatest() async {
      if (!itemScrollController.isAttached) return;
      final lastIndex = replies.isEmpty
          ? headIndex
          : indexForReply(replies.length - 1);
      followsThreadTail.value = true;
      pendingTailAlignment.value = null;
      await itemScrollController.scrollTo(
        index: lastIndex,
        duration: const Duration(milliseconds: 220),
        curve: Curves.easeOutCubic,
      );
    }

    useEffect(() {
      final messageId = initialMessageId;
      // Wait for the authoritative thread query before consuming the one-shot
      // jump; the fallback main-timeline list can contain only the linked reply.
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
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (!context.mounted || !itemScrollController.isAttached) return;
        // The provisional route snapshot can make the linked reply look like
        // the tail. This authoritative deep-link jump intentionally leaves
        // the user at an older item, so it must opt out of follow-tail first.
        followsThreadTail.value = false;
        pendingTailAlignment.value = null;
        itemScrollController.jumpTo(index: targetIndex, alignment: 0.35);
        didJumpToInitialMessage.value = true;
      });
      return null;
    }, [initialMessageId, fetchedReplies, replies.length]);

    // A top-anchored list doesn't stick to the newest item the way the old
    // reversed one did, so follow the tail explicitly: when a reply arrives
    // while the last item is on screen, scroll it into view. If the user has
    // scrolled up to read, leave them where they are.
    final hasFetchedReplies = fetchedReplies != null;
    final didEstablishInitialReplies = useRef(hasFetchedReplies);
    final previousReplyCount = useRef(replies.length);
    useEffect(() {
      // The first authoritative query result is hydration, not a live arrival.
      // Establish the baseline without moving the user away from the head.
      if (!hasFetchedReplies) return null;
      if (!didEstablishInitialReplies.value) {
        didEstablishInitialReplies.value = true;
        previousReplyCount.value = replies.length;
        return null;
      }

      final previous = previousReplyCount.value;
      previousReplyCount.value = replies.length;
      if (replies.length <= previous) return null;
      final positions = itemPositionsListener.itemPositions.value;
      final lastIndex = indexForReply(replies.length - 1);
      // Positions still describe the list as it was *before* these replies, so
      // compare against the old tail. Measuring against the new one only reads
      // as "at the tail" when exactly one reply arrived.
      final previousLastIndex = previous == 0
          ? headIndex
          : indexForReply(previous - 1);
      final wasAtTail =
          positions.isEmpty ||
          positions.any((position) => position.index >= previousLastIndex);
      final localPubkey = currentPubkey?.toLowerCase();
      final hasNewLocalReply =
          localPubkey != null &&
          replies
              .skip(previous)
              .any((reply) => reply.pubkey.toLowerCase() == localPubkey);
      // A reply the current user just sent must be visible even if they were
      // reading at the head of a long thread. Remote arrivals still respect
      // the user's scroll position.
      if (!wasAtTail && !hasNewLocalReply) return null;
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (!context.mounted || !itemScrollController.isAttached) return;
        itemScrollController.scrollTo(
          index: lastIndex,
          duration: const Duration(milliseconds: 220),
          curve: Curves.easeOutCubic,
        );
      });
      return null;
    }, [hasFetchedReplies, replies.length]);
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
          () => readState.effectiveTimestamp(msgContextKey(reply.id)),
        );
      }
    }

    // Oldest reply the user has not seen, from the frozen snapshot. Own
    // replies never count as unread.
    String? firstUnreadReplyId;
    if (readState.isReady) {
      final localPubkey = currentPubkey?.toLowerCase();
      for (final reply in replies) {
        if (localPubkey != null && reply.pubkey.toLowerCase() == localPubkey) {
          continue;
        }
        final snapshot = openReadSnapshot.value;
        if (!snapshot.containsKey(reply.id)) continue;
        final readAt = snapshot[reply.id];
        if (readState.isForcedUnread(msgContextKey(reply.id)) ||
            readAt == null ||
            reply.createdAt > readAt) {
          firstUnreadReplyId = reply.id;
          break;
        }
      }
    }

    // One-shot resume: land on the first unread reply instead of the tail.
    // A deep link names its own target and always wins.
    final didResumeAtFirstUnread = useRef(false);
    final resumeTargetId = firstUnreadReplyId;
    useEffect(() {
      if (initialMessageId != null || fetchedReplies == null) return null;
      if (didResumeAtFirstUnread.value || resumeTargetId == null) return null;
      final chronologicalIndex = replies.indexWhere(
        (reply) => reply.id == resumeTargetId,
      );
      if (chronologicalIndex < 0) return null;
      final targetIndex = indexForReply(chronologicalIndex);
      didResumeAtFirstUnread.value = true;
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (!context.mounted || !itemScrollController.isAttached) return;
        // Resuming deliberately leaves the reader above the newest reply, so
        // opt out of tail-following before moving.
        followsThreadTail.value = false;
        pendingTailAlignment.value = null;
        itemScrollController.jumpTo(index: targetIndex, alignment: 0.35);
      });
      return null;
    }, [initialMessageId, fetchedReplies, replies.length, resumeTargetId]);

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

    // Thread-scoped typing indicators (exclude self).
    final allTyping = ref.watch(channelTypingProvider(channelId));
    final threadTyping = allTyping
        .where((e) => e.threadHeadId == threadHead.id)
        .where(
          (e) =>
              currentPubkey == null ||
              e.pubkey.toLowerCase() != currentPubkey?.toLowerCase(),
        )
        .toList();

    // Resolve thread head from live data (reactions/edits may have changed).
    final liveHead =
        allMsgs.where((m) => m.id == threadHead.id).firstOrNull ?? threadHead;

    // The root of the entire thread chain. If the current thread head is
    // itself a root message its rootId is null, so fall back to its own id.
    final effectiveRootId = threadHead.rootId ?? threadHead.id;

    void updateComposerDockHeight(double height) {
      final previousHeight = composerDockHeight.value;
      final heightDelta = height - previousHeight;
      if (heightDelta.abs() < 0.5) return;

      final shouldFollowTail = followsThreadTail.value || threadTailIsVisible();
      if (shouldFollowTail) followsThreadTail.value = true;
      composerDockHeight.value = height;
      if (heightDelta <= 0 || !shouldFollowTail) {
        pendingTailAlignment.value = null;
        return;
      }
      final lastIndex = replies.isEmpty
          ? headIndex
          : indexForReply(replies.length - 1);
      final lastPosition = itemPositionsListener.itemPositions.value
          .where((position) => position.index == lastIndex)
          .firstOrNull;
      if (lastPosition == null) return;
      final targetAlignment =
          (pendingTailAlignment.value ?? lastPosition.itemLeadingEdge) -
          (heightDelta / MediaQuery.sizeOf(context).height);
      pendingTailAlignment.value = targetAlignment;

      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (!context.mounted || !itemScrollController.isAttached) return;
        itemScrollController.jumpTo(
          index: lastIndex,
          alignment: targetAlignment,
        );
      });
    }

    // Composer size changes and keyboard metrics changes are independent:
    // the dock grows first, then the Scaffold's viewport shrinks once the
    // keyboard appears. Re-align after that latter layout pass too, but only
    // while the user was already following the thread tail.
    void realignThreadTailAfterMetricsChange() {
      final shouldFollowTail = followsThreadTail.value || threadTailIsVisible();
      if (!shouldFollowTail || tailRealignmentQueued.value) return;
      followsThreadTail.value = true;
      tailRealignmentQueued.value = true;
      WidgetsBinding.instance.addPostFrameCallback((_) {
        tailRealignmentQueued.value = false;
        if (!context.mounted ||
            !itemScrollController.isAttached ||
            !followsThreadTail.value) {
          return;
        }
        final lastIndex = replies.isEmpty
            ? headIndex
            : indexForReply(replies.length - 1);
        itemScrollController.scrollTo(
          index: lastIndex,
          duration: const Duration(milliseconds: 220),
          curve: Curves.easeOutCubic,
        );
      });
    }

    useEffect(() {
      final observer = _ThreadTailMetricsObserver(
        onMetricsChanged: realignThreadTailAfterMetricsChange,
      );
      WidgetsBinding.instance.addObserver(observer);
      return () => WidgetsBinding.instance.removeObserver(observer);
    }, [itemScrollController, replies.length]);

    // Channel names for message content rendering.
    final channelsAsync = ref.watch(channelsProvider);
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
                child: KeyboardDismissOnDrag(
                  onUserScrollStart: () {
                    followsThreadTail.value = false;
                    pendingTailAlignment.value = null;
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
                          key: ValueKey('thread-message-group-${liveHead.id}'),
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
                                isHighlighted: liveHead.id == initialMessageId,
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
                                            color:
                                                context.colors.onSurfaceVariant,
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
              if (!isMember || isArchived)
                AnimatedSize(
                  duration: MediaQuery.disableAnimationsOf(context)
                      ? Duration.zero
                      : const Duration(milliseconds: 180),
                  curve: Curves.easeOutCubic,
                  alignment: Alignment.bottomCenter,
                  child: threadTyping.isEmpty
                      ? const SizedBox.shrink()
                      : ChannelTypingIndicator(entries: threadTyping),
                ),
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
                    AnimatedSize(
                      duration: MediaQuery.disableAnimationsOf(context)
                          ? Duration.zero
                          : const Duration(milliseconds: 180),
                      curve: Curves.easeOutCubic,
                      alignment: Alignment.bottomCenter,
                      child: threadTyping.isEmpty
                          ? const SizedBox.shrink()
                          : ChannelTypingIndicator(entries: threadTyping),
                    ),
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
                            parentEventId: threadHead.id,
                            rootEventId: effectiveRootId,
                            mediaTags: mediaTags,
                          ),
                    ),
                  ],
                ),
              ),
            ),
          if (!isAtThreadTail.value)
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

bool _isDeletedBy(Iterable<NostrEvent> events, String messageId) {
  for (final event in events) {
    if (event.kind != EventKind.deletion &&
        event.kind != EventKind.nip29DeleteEvent) {
      continue;
    }
    if (event.tags.any(
      (tag) => tag.length >= 2 && tag[0] == 'e' && tag[1] == messageId,
    )) {
      return true;
    }
  }
  return false;
}

/// Build a lightweight summary for a nested thread (reply that has its own
/// replies). Same logic as the top-level [ThreadSummary] but kept local to
/// avoid coupling.
ThreadSummary _buildNestedSummary(
  String messageId,
  List<TimelineMessage> children,
) {
  final seen = <String>{};
  final participants = <String>[];
  for (var i = children.length - 1; i >= 0 && participants.length < 3; i--) {
    final pk = children[i].pubkey.toLowerCase();
    if (seen.add(pk)) participants.add(pk);
  }
  return ThreadSummary(
    threadHeadId: messageId,
    replyCount: children.length,
    participantPubkeys: participants.reversed.toList(),
    lastReplyAt: children.last.createdAt,
  );
}

class _ThreadTailMetricsObserver with WidgetsBindingObserver {
  final VoidCallback onMetricsChanged;

  _ThreadTailMetricsObserver({required this.onMetricsChanged});

  @override
  void didChangeMetrics() => onMetricsChanged();
}
