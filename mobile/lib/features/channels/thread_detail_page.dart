import 'dart:async';

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
import 'initial_thread_tail_settle.dart';
import 'laid_out_viewport.dart';
import 'latest_message_button.dart';
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

part 'thread_detail_page/nested_thread_summary_row.dart';
part 'thread_detail_helpers.dart';
part 'thread_detail_page/tail_alignment.dart';
part 'thread_detail_page/thread_message.dart';
part 'thread_detail_page/avatar.dart';

const _landingHighlightDuration = Duration(seconds: 3);
const _landingHighlightDelay = Duration(milliseconds: 50);
const _landingHighlightTransitionDuration = Duration(milliseconds: 300);
const _landingHighlightOpacity = 0.12;

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
    final composerFocusNode = useFocusNode();
    final restoreComposerFocus = useRef<VoidCallback?>(null);
    final settledImeBottomInset = useState(
      usesFixedAndroidImeViewport
          ? appView.viewInsets.bottom / appView.devicePixelRatio
          : 0.0,
    );
    useEffect(() {
      final session = ref.read(relaySessionProvider.notifier);
      return session.registerVisibleChannel(channelId);
    }, [channelId]);
    final sendMessage = ref.read(sendMessageProvider);
    // Relay thread queries are keyed by the outermost root, even when this
    // page displays a nested branch. Query that root, then select this head's
    // direct children from the returned subtree below.
    final queryRootId = threadHead.rootId ?? threadHead.id;
    final repliesArgs = ThreadRepliesArgs(
      channelId: channelId,
      rootId: queryRootId,
    );
    final relayReplyState = ref.watch(threadRepliesProvider(repliesArgs));
    final repliesState = ref.watch(threadRepliesWithLocalProvider(repliesArgs));
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
    // A terminal query error cannot produce a more authoritative list. Keep
    // loading states provisional, but let the hydrated route snapshot drive
    // the one-shot target jump when the relay query has definitively failed.
    final canUseMessagesForInitialTarget =
        relayReplyState.value != null ||
        (relayReplyState.hasError && !relayReplyState.retrying);
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
    final routeAnimation = ModalRoute.of(context)?.animation;
    final reducedLandingHighlightMotion = MediaQuery.disableAnimationsOf(
      context,
    );
    final highlightedMessageId = useState<String?>(null);
    final initialTargetReadyForHighlight = useState(false);
    useEffect(
      () {
        final messageId = initialMessageId;
        if (messageId == null || !initialTargetReadyForHighlight.value) {
          return null;
        }
        var disposed = false;
        Timer? revealTimer;
        Timer? dismissTimer;

        void revealHighlight() {
          WidgetsBinding.instance.addPostFrameCallback((_) {
            if (disposed) return;
            revealTimer = Timer(_landingHighlightDelay, () {
              if (disposed) return;
              highlightedMessageId.value = messageId;
              dismissTimer = Timer(
                _landingHighlightDuration +
                    (reducedLandingHighlightMotion
                        ? Duration.zero
                        : _landingHighlightTransitionDuration),
                () {
                  if (!disposed) highlightedMessageId.value = null;
                },
              );
            });
          });
        }

        void handleRouteStatus(AnimationStatus status) {
          if (status != AnimationStatus.completed) return;
          routeAnimation?.removeStatusListener(handleRouteStatus);
          revealHighlight();
        }

        if (routeAnimation == null ||
            routeAnimation.status == AnimationStatus.completed) {
          revealHighlight();
        } else {
          routeAnimation.addStatusListener(handleRouteStatus);
        }

        return () {
          disposed = true;
          routeAnimation?.removeStatusListener(handleRouteStatus);
          revealTimer?.cancel();
          dismissTimer?.cancel();
        };
      },
      [
        initialMessageId,
        initialTargetReadyForHighlight.value,
        reducedLandingHighlightMotion,
        routeAnimation,
      ],
    );

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
    final listViewport = useMemoized(LaidOutViewport.new);
    useEffect(() => listViewport.dispose, [listViewport]);
    final didJumpToInitialMessage = useRef(false);
    final initialHighlightTargetIndex = useState<int?>(null);
    final followsThreadTail = useRef(false);
    final userOptedOutOfTailFollow = useRef(false);
    final userDragDetachedTailFollow = useRef(false);
    final tailIntent = useMemoized(_ThreadTailIntent.new);
    final initialTailSettle = useMemoized(InitialThreadTailSettle.new);
    final isAtThreadTail = useState(true);
    final tailCorrectionInProgress = useRef(false);
    final viewportHeight = useListenable(listViewport.height).value;
    final previousViewportHeight = useRef(viewportHeight);
    final settledImeLift = usesFixedAndroidImeViewport
        ? (settledImeBottomInset.value -
                  MediaQuery.viewPaddingOf(context).bottom)
              .clamp(0.0, double.infinity)
              .toDouble()
        : 0.0;
    final timelineBottomInset =
        composerDockHeight.value +
        (followsThreadTail.value || !initialTailSettle.isComplete
            ? settledImeLift
            : 0);
    final navigationBottomInset = composerDockHeight.value + settledImeLift;

    // Item 0 is the thread head; reply `i` lives at `i + 1`.
    const headIndex = 0;
    int indexForReply(int chronologicalIndex) => chronologicalIndex + 1;
    final tailAnchorIndex = replies.length + 1;

    double threadTailAlignment() => _threadTailAlignmentForViewport(
      // This reporter already reflects Scaffold resize, typing rows, and every
      // other layout consumer above or below the list.
      fullHeight: viewportHeight > 0
          ? viewportHeight
          : MediaQuery.sizeOf(context).height,
      imeBottomInset: 0,
      usesFixedImeViewport: true,
      bottomInset:
          Grid.xs +
          composerDockHeight.value +
          (followsThreadTail.value ? settledImeLift : 0),
    );

    bool threadTailIsVisible() {
      if (isMember &&
          !isArchived &&
          (!viewportHeight.isFinite ||
              viewportHeight <= 0 ||
              composerDockHeight.value <= 0)) {
        return false;
      }
      if (!viewportHeight.isFinite || viewportHeight <= 0) return false;
      final trailingBoundary =
          1 - ((Grid.xs + timelineBottomInset) / viewportHeight) + 0.001;
      final lastMessageIndex = _threadTailIndex(replies.length);
      return itemPositionsListener.itemPositions.value.any(
        (position) =>
            position.index == lastMessageIndex &&
            position.itemTrailingEdge <= trailingBoundary,
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
      if (userDragDetachedTailFollow.value) return;
      initialTailSettle.abandon();
      tailIntent.endDrag();
      tailIntent.detach();
      userOptedOutOfTailFollow.value = false;
      followsThreadTail.value = true;
      isAtThreadTail.value = true;
      if (!threadTailIsVisible()) correctThreadTailInstantly();
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

    Future<void> scrollToThreadLatest() async {
      if (!itemScrollController.isAttached) return;
      initialTailSettle.abandon();
      tailIntent.endDrag();
      userOptedOutOfTailFollow.value = false;
      userDragDetachedTailFollow.value = false;
      followsThreadTail.value = true;
      isAtThreadTail.value = true;
      tailIntent.scheduleNextFrame(
        allowed: true,
        revalidate: () =>
            context.mounted &&
            itemScrollController.isAttached &&
            !tailIntent.isDragging,
        action: () async {
          await itemScrollController.scrollTo(
            index: tailAnchorIndex,
            alignment: threadTailAlignment(),
            duration: const Duration(milliseconds: 220),
            curve: Curves.easeOutCubic,
          );
          if (context.mounted && threadTailIsVisible()) {
            isAtThreadTail.value = true;
          }
        },
      );
    }

    useEffect(
      () {
        final messageId = initialMessageId;
        // Wait for either the authoritative thread query or a terminal query
        // error before consuming the one-shot jump. During loading, the fallback
        // main-timeline list can contain only the linked reply; after an error,
        // that hydrated snapshot is the best available target list.
        if (messageId == null || !canUseMessagesForInitialTarget) return null;
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
        initialTailSettle.abandon();
        userOptedOutOfTailFollow.value = true;
        userDragDetachedTailFollow.value = false;
        tailIntent.schedule(
          allowed: true,
          revalidate: () =>
              context.mounted &&
              itemScrollController.isAttached &&
              !tailIntent.isDragging,
          action: () {
            // The provisional route snapshot can make the linked reply look like
            // the tail. This authoritative deep-link jump intentionally leaves
            // the user at an older item, so it must opt out of follow-tail first.
            tailIntent.detach();
            followsThreadTail.value = false;
            isAtThreadTail.value = false;
            itemScrollController.jumpTo(index: targetIndex, alignment: 0.35);
            initialHighlightTargetIndex.value = targetIndex;
          },
        );
        return null;
      },
      [
        initialMessageId,
        canUseMessagesForInitialTarget,
        fetchedReplies,
        replies.length,
      ],
    );

    useEffect(
      () {
        final targetIndex = initialHighlightTargetIndex.value;
        if (targetIndex == null || initialTargetReadyForHighlight.value) {
          return null;
        }
        var completionScheduled = false;
        void markReadyAfterTargetLayout() {
          if (completionScheduled ||
              !itemPositionsListener.itemPositions.value.any(
                (position) => position.index == targetIndex,
              )) {
            return;
          }
          completionScheduled = true;
          WidgetsBinding.instance.addPostFrameCallback((_) {
            if (context.mounted) initialTargetReadyForHighlight.value = true;
          });
        }

        itemPositionsListener.itemPositions.addListener(
          markReadyAfterTargetLayout,
        );
        markReadyAfterTargetLayout();
        return () => itemPositionsListener.itemPositions.removeListener(
          markReadyAfterTargetLayout,
        );
      },
      [initialHighlightTargetIndex.value, initialTargetReadyForHighlight.value],
    );

    // A top-anchored list doesn't stick to the newest item the way the old
    // reversed one did, so follow the tail explicitly: when a reply arrives
    // while the last item is on screen, scroll it into view. If the user has
    // scrolled up to read, leave them where they are.
    final hasFetchedReplies = fetchedReplies != null;
    final previousReplyCount = useRef(replies.length);
    final topOverlayFraction = viewportHeight > 0
        ? frostedAppBarHeight(context) / viewportHeight
        : 0.0;
    final settleGeometry = (
      composerDockHeight.value,
      settledImeLift,
      viewportHeight,
    );
    useEffect(() {
      if (!hasFetchedReplies || viewportHeight <= 0) return null;
      if (initialMessageId != null) {
        initialTailSettle.abandon();
        previousReplyCount.value = replies.length;
        return null;
      }
      if (isMember && !isArchived && composerDockHeight.value <= 0) {
        return null;
      }
      if (!initialTailSettle.isComplete) {
        previousReplyCount.value = replies.length;
        followsThreadTail.value = true;
        initialTailSettle.schedule(
          context: context,
          controller: itemScrollController,
          positionsListener: itemPositionsListener,
          targetIndex: replies.isEmpty
              ? null
              : indexForReply(replies.length - 1),
          hiddenTopFraction: topOverlayFraction,
          hiddenBottomFraction:
              (composerDockHeight.value + settledImeLift) / viewportHeight,
        );
        return null;
      }

      final previous = previousReplyCount.value;
      previousReplyCount.value = replies.length;
      if (replies.length <= previous) return null;
      final positions = itemPositionsListener.itemPositions.value;
      // Positions still describe the list as it was *before* these replies, so
      // compare against the old tail. Measuring against the new one only reads
      // as "at the tail" when exactly one reply arrived.
      final previousTailAnchorIndex = previous + 1;
      final wasAtTail =
          positions.isEmpty ||
          positions.any(
            (position) => position.index >= previousTailAnchorIndex,
          );
      final localPubkey = currentPubkey?.toLowerCase();
      final hasNewLocalReply =
          localPubkey != null &&
          replies
              .skip(previous)
              .any((reply) => reply.pubkey.toLowerCase() == localPubkey);
      // A reply the current user just sent must be visible even if they were
      // reading at the head of a long thread. Remote arrivals still respect
      // the user's scroll position.
      if (tailIntent.isDragging) return null;
      if (!hasNewLocalReply && (userOptedOutOfTailFollow.value || !wasAtTail)) {
        return null;
      }
      if (hasNewLocalReply) {
        userOptedOutOfTailFollow.value = false;
        userDragDetachedTailFollow.value = false;
      }
      followsThreadTail.value = true;
      tailIntent.scheduleNextFrame(
        allowed: true,
        revalidate: () =>
            context.mounted &&
            itemScrollController.isAttached &&
            !tailIntent.isDragging &&
            !userOptedOutOfTailFollow.value,
        // A reply arrival changes list geometry. Keep this correction instant;
        // only an explicit tap on Latest should animate navigation.
        action: correctThreadTailInstantly,
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

    // Composer size changes and keyboard metrics changes are independent:
    // the dock grows first, then the Scaffold's viewport shrinks once the
    // keyboard appears. Re-align after that latter layout pass too, but only
    // while the user was already following the thread tail.
    void realignThreadTailAfterMetricsChange() {
      listViewport.reportAfterLayout();
      final shouldFollowTail =
          !tailIntent.isDragging &&
          !userOptedOutOfTailFollow.value &&
          (followsThreadTail.value || threadTailIsVisible());
      if (!shouldFollowTail || !initialTailSettle.isComplete) return;
      followsThreadTail.value = true;
      tailIntent.scheduleNextFrame(
        allowed: true,
        revalidate: () =>
            context.mounted &&
            itemScrollController.isAttached &&
            !tailIntent.isDragging &&
            !userOptedOutOfTailFollow.value &&
            followsThreadTail.value,
        action: () {
          final targetAlignment = threadTailAlignment();
          final positions = itemPositionsListener.itemPositions.value;
          final anchorPosition = positions
              .where((position) => position.index == tailAnchorIndex)
              .firstOrNull;
          final headIsVisible = positions.any(
            (position) =>
                position.index == headIndex && position.itemTrailingEdge > 0,
          );
          if (anchorPosition != null &&
              ((anchorPosition.itemLeadingEdge - targetAlignment).abs() <
                      0.005 ||
                  (headIsVisible &&
                      anchorPosition.itemLeadingEdge < targetAlignment))) {
            return;
          }
          // This runs once after Android's frame-by-frame IME metrics settle.
          // Keep the resulting layout correction instant.
          correctThreadTailInstantly();
        },
      );
    }

    useEffect(() {
      final previousHeight = previousViewportHeight.value;
      previousViewportHeight.value = viewportHeight;
      if ((viewportHeight - previousHeight).abs() >= 0.5 &&
          initialTailSettle.isComplete) {
        realignThreadTailAfterMetricsChange();
      }
      return null;
    }, [viewportHeight]);

    void updateComposerDockHeight(double height) {
      listViewport.reportAfterLayout();
      final previousHeight = composerDockHeight.value;
      final heightDelta = height - previousHeight;
      if (heightDelta.abs() < 0.5) return;

      final shouldFollowTail =
          !tailIntent.isDragging &&
          !userOptedOutOfTailFollow.value &&
          (followsThreadTail.value || threadTailIsVisible());
      if (shouldFollowTail) followsThreadTail.value = true;
      composerDockHeight.value = height;
      if (shouldFollowTail) realignThreadTailAfterMetricsChange();
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
    }, [appView, itemScrollController]);

    useEffect(() {
      if (usesFixedAndroidImeViewport) {
        realignThreadTailAfterMetricsChange();
      }
      return null;
    }, [settledImeBottomInset.value]);

    // Channel names for message content rendering.
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
                      userDragDetachedTailFollow.value = true;
                      followsThreadTail.value = false;
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
                          if (!userOptedOutOfTailFollow.value) {
                            userDragDetachedTailFollow.value = false;
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
                                      liveHead.id == highlightedMessageId.value,
                                  allMessages: allMsgs,
                                  isMember: isMember,
                                  isArchived: isArchived,
                                  isThreadHead: true,
                                  composerFocusNode: composerFocusNode,
                                  restoreComposerFocus: () =>
                                      restoreComposerFocus.value?.call(),
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
                                isHighlighted:
                                    reply.id == highlightedMessageId.value,
                                allMessages: allMsgs,
                                isMember: isMember,
                                isArchived: isArchived,
                                composerFocusNode: composerFocusNode,
                                restoreComposerFocus: () =>
                                    restoreComposerFocus.value?.call(),
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
                        focusNode: composerFocusNode,
                        onFocusRestorerChanged: (restoreFocus) =>
                            restoreComposerFocus.value = restoreFocus,
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
          if (hasFetchedReplies && !isAtThreadTail.value)
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
