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

    bool threadTailIsVisible() {
      final lastIndex = _threadTailIndex(replies.length);
      final trailingBoundary = _threadTailTrailingBoundary(
        hasComposerDock: isMember && !isArchived,
        viewportHeight: listViewport.height.value,
        dockHeight: composerDockHeight.value,
      );
      return itemPositionsListener.itemPositions.value.any(
        (position) =>
            position.index == lastIndex &&
            position.itemTrailingEdge <= trailingBoundary,
      );
    }

    useEffect(() {
      void onPositionsChanged() {
        if (!userOptedOutOfTailFollow.value && threadTailIsVisible()) {
          followsThreadTail.value = true;
        }
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
                            parentEventId: threadHead.id,
                            rootEventId: effectiveRootId,
                            mediaTags: mediaTags,
                          ),
                    ),
                  ],
                ),
              ),
            ),
        ],
      ),
    );
  }
}

/// Tappable summary row shown below a reply that itself has replies.
/// Pushes a new [ThreadDetailPage] for the nested thread.
class _NestedThreadSummaryRow extends ConsumerWidget {
  final ThreadSummary summary;
  final TimelineMessage replyMessage;
  final List<TimelineMessage> allMessages;
  final String channelId;
  final String? currentPubkey;
  final bool isMember;
  final bool isArchived;

  const _NestedThreadSummaryRow({
    required this.summary,
    required this.replyMessage,
    required this.allMessages,
    required this.channelId,
    required this.currentPubkey,
    required this.isMember,
    required this.isArchived,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final userCache = ref.watch(userCacheProvider);

    return GestureDetector(
      onTap: () {
        Navigator.of(context).push(
          MaterialPageRoute<void>(
            builder: (_) => ThreadDetailPage(
              threadHead: replyMessage,
              allMessages: allMessages,
              channelId: channelId,
              currentPubkey: currentPubkey,
              isMember: isMember,
              isArchived: isArchived,
            ),
          ),
        );
      },
      child: Padding(
        key: ValueKey('nested-thread-summary-${replyMessage.id}'),
        padding: const EdgeInsets.only(
          left: messageAvatarSize + messageAvatarContentGap,
          top: Grid.half,
          bottom: Grid.xs,
        ),
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            // Stacked participant avatars.
            SizedBox(
              width:
                  32.0 +
                  (summary.participantPubkeys.length - 1).clamp(0, 2) * 20.0,
              height: 32,
              child: Stack(
                children: [
                  for (var i = 0; i < summary.participantPubkeys.length; i++)
                    Positioned(
                      left: i * 20.0,
                      child: SmallAvatar(
                        pubkey: summary.participantPubkeys[i],
                        userCache: userCache,
                        size: 32,
                      ),
                    ),
                ],
              ),
            ),
            const SizedBox(width: Grid.xxs),
            Flexible(
              child: Text.rich(
                TextSpan(
                  children: [
                    TextSpan(
                      text:
                          '${summary.replyCount} ${summary.replyCount == 1 ? 'reply' : 'replies'}',
                      style: replyPreviewTextStyle.copyWith(
                        color: context.colors.primary,
                      ),
                    ),
                    if (summary.lastReplyAt case final lastReplyAt?) ...[
                      TextSpan(
                        text: ' · ',
                        style: replyPreviewTextStyle.copyWith(
                          color: context.colors.onSurfaceVariant.withValues(
                            alpha: 0.5,
                          ),
                        ),
                      ),
                      TextSpan(
                        text:
                            'last reply ${formatThreadSummaryLastReplyTime(lastReplyAt)}',
                        style: replyPreviewTextStyle.copyWith(
                          color: context.colors.onSurfaceVariant,
                        ),
                      ),
                    ],
                  ],
                ),
                maxLines: 2,
                overflow: TextOverflow.ellipsis,
              ),
            ),
          ],
        ),
      ),
    );
  }
}

class _ThreadMessage extends ConsumerWidget {
  final TimelineMessage message;
  final Map<String, String> channelNames;
  final String channelId;
  final String? currentPubkey;
  final bool showAuthor;
  final bool isHighlighted;
  final List<TimelineMessage>? allMessages;
  final bool isMember;
  final bool isArchived;

  /// Whether this is the message the thread hangs off, which keeps a standing
  /// "+" where replies only get one once they carry a reaction.
  final bool isThreadHead;

  const _ThreadMessage({
    required this.message,
    required this.channelNames,
    required this.channelId,
    required this.currentPubkey,
    required this.showAuthor,
    this.isHighlighted = false,
    this.allMessages,
    this.isMember = false,
    this.isArchived = false,
    this.isThreadHead = false,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final pk = message.pubkey.toLowerCase();
    final profile =
        ref.watch(userCacheProvider.select((cache) => cache[pk])) ??
        ref.read(userCacheProvider.notifier).get(pk);
    final displayName = profile?.label ?? shortPubkey(message.pubkey);
    final canManageMessage =
        currentPubkey?.toLowerCase() == pk ||
        (profile?.ownerPubkey != null &&
            profile?.ownerPubkey == currentPubkey?.toLowerCase());

    final userCache = ref.watch(userCacheProvider);
    final knownAgentPubkeys = agentPubkeysWithProfileOwners(
      knownAgentPubkeys: ref.watch(agentMentionPubkeysProvider(channelId)),
      profileOwnedAgentPubkeys: [
        for (final profile in userCache.values)
          if (profile.ownerPubkey != null) profile.pubkey,
      ],
    );
    final mentionNames = <String, String>{};
    final agentMentionPubkeys = <String>{};
    for (final mpk in message.mentionPubkeys) {
      final normalizedPubkey = mpk.toLowerCase();
      final p = userCache[normalizedPubkey];
      if (p?.displayName != null) {
        mentionNames[normalizedPubkey] = p!.displayName!;
      }
      if (knownAgentPubkeys.contains(normalizedPubkey)) {
        agentMentionPubkeys.add(normalizedPubkey);
      }
    }
    final resolvedMentionNames = mentionNamesWithDirectoryLabels(
      mentionPubkeys: message.mentionPubkeys,
      profileMentionNames: mentionNames,
      directoryDisplayNames: ref.watch(agentDirectoryDisplayNamesProvider),
      agentMentionPubkeys: agentMentionPubkeys,
    );

    void openMessageActions(Rect anchorRect) {
      showMessageActions(
        context: context,
        ref: ref,
        message: message,
        channelId: channelId,
        canManageMessage: canManageMessage,
        allMessages: allMessages,
        currentPubkey: currentPubkey,
        isMember: isMember,
        isArchived: isArchived,
        anchorRect: anchorRect,
      );
    }

    return Padding(
      padding: EdgeInsets.only(top: showAuthor ? Grid.xs : 0),
      child: DecoratedBox(
        key: ValueKey('thread-message-${message.id}'),
        decoration: BoxDecoration(
          color: isHighlighted
              ? context.colors.primary.withValues(alpha: 0.12)
              : Colors.transparent,
          borderRadius: BorderRadius.circular(Radii.md),
        ),
        child: Material(
          color: Colors.transparent,
          borderRadius: BorderRadius.circular(Radii.md),
          // The media carousel intentionally continues through the list's
          // trailing gutter. InkWell still clips its ink to [borderRadius],
          // while leaving overflowing message content visible.
          clipBehavior: Clip.none,
          child: MessageLongPressInkWell(
            key: ValueKey('thread-message-row-${message.id}'),
            onLongPress: openMessageActions,
            borderRadius: BorderRadius.circular(Radii.md),
            highlightColor: context.colors.primary.withValues(alpha: 0.1),
            child: Padding(
              padding: EdgeInsets.only(
                top: showAuthor ? 0 : Grid.xxs,
                bottom: showAuthor ? 0 : Grid.xxs,
              ),
              child: Row(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  if (showAuthor)
                    GestureDetector(
                      onTap: () =>
                          showUserProfileSheet(context, message.pubkey),
                      child: _Avatar(profile: profile, pubkey: message.pubkey),
                    )
                  else
                    const SizedBox(width: messageAvatarSize),
                  const SizedBox(width: messageAvatarContentGap),
                  Expanded(
                    child: Padding(
                      padding: EdgeInsets.only(top: showAuthor ? Grid.half : 0),
                      child: Column(
                        crossAxisAlignment: CrossAxisAlignment.start,
                        children: [
                          if (showAuthor)
                            Padding(
                              padding: const EdgeInsets.only(
                                bottom: Grid.quarter,
                              ),
                              child: Row(
                                children: [
                                  Expanded(
                                    child: MessageAuthorMeta(
                                      displayName: displayName,
                                      username: messageUsernameLabel(profile),
                                      timestamp: formatMessageTime(
                                        message.createdAt,
                                      ),
                                      nameColor: context.colors.onSurface,
                                      metadataColor:
                                          context.colors.onSurfaceVariant,
                                      onAuthorTap: () => showUserProfileSheet(
                                        context,
                                        message.pubkey,
                                      ),
                                      displayNameKey: ValueKey(
                                        'thread-message-author-${message.id}',
                                      ),
                                      usernameKey: ValueKey(
                                        'thread-message-username-${message.id}',
                                      ),
                                      timestampKey: ValueKey(
                                        'thread-message-timestamp-${message.id}',
                                      ),
                                    ),
                                  ),
                                  if (message.edited) ...[
                                    const SizedBox(width: Grid.half),
                                    Text(
                                      '(edited)',
                                      style: context.textTheme.labelSmall
                                          ?.copyWith(
                                            color:
                                                context.colors.onSurfaceVariant,
                                            fontStyle: FontStyle.italic,
                                          ),
                                    ),
                                  ],
                                ],
                              ),
                            ),
                          MessageContent(
                            content: message.content,
                            mentionNames: resolvedMentionNames,
                            agentMentionPubkeys: agentMentionPubkeys,
                            channelNames: channelNames,
                            tags: message.tags,
                            baseStyle: messageBodyTextStyle.copyWith(
                              color: context.colors.onSurface,
                            ),
                            scaleEmojiOnly: true,
                            mediaCarouselTrailingOverflow: Grid.gutter,
                            onMediaReply: allMessages == null
                                ? null
                                : () {
                                    if (!context.mounted) return;
                                    Navigator.of(context).push(
                                      MaterialPageRoute<void>(
                                        builder: (_) => ThreadDetailPage(
                                          threadHead: message,
                                          allMessages: allMessages!,
                                          channelId: channelId,
                                          currentPubkey: currentPubkey,
                                          isMember: isMember,
                                          isArchived: isArchived,
                                        ),
                                      ),
                                    );
                                  },
                            onMediaMore: (viewerContext, imageUrl) =>
                                showImageActions(
                                  context: viewerContext,
                                  ref: ref,
                                  message: message,
                                  channelId: channelId,
                                  imageUrl: imageUrl,
                                  canManageMessage: canManageMessage,
                                  onDeleted: () {
                                    if (viewerContext.mounted) {
                                      Navigator.of(viewerContext).maybePop();
                                    }
                                  },
                                ),
                            onChannelTap: (targetChannelId) {
                              openChannelLink(
                                context: context,
                                ref: ref,
                                channelId: targetChannelId,
                                currentChannelId: channelId,
                              );
                            },
                            onMentionTap: (pubkey) =>
                                showUserProfileSheet(context, pubkey),
                          ),
                          ReactionRow(
                            messageId: message.id,
                            reactions: message.reactions,
                            onToggle: (emoji) =>
                                toggleReaction(ref, message, emoji),
                            showAddButton:
                                isMember &&
                                !isArchived &&
                                (isThreadHead || message.reactions.isNotEmpty),
                            onAddReaction: () => showAddReactionPicker(
                              context: context,
                              ref: ref,
                              message: message,
                            ),
                          ),
                        ],
                      ),
                    ),
                  ),
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }
}

class _Avatar extends StatelessWidget {
  final UserProfile? profile;
  final String pubkey;

  const _Avatar({required this.profile, required this.pubkey});

  @override
  Widget build(BuildContext context) {
    final initial =
        profile?.initial ?? (pubkey.isNotEmpty ? pubkey[0].toUpperCase() : '?');
    final avatarUrl = profile?.avatarUrl;

    return AvatarImage(
      imageUrl: avatarUrl,
      radius: messageAvatarSize / 2,
      backgroundColor: context.colors.primaryContainer,
      fallback: Text(
        initial,
        style: context.textTheme.labelMedium?.copyWith(
          color: context.colors.onPrimaryContainer,
          fontWeight: FontWeight.w600,
        ),
      ),
    );
  }
}
