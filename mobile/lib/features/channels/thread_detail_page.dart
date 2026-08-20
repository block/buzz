import 'dart:async';
import 'dart:math' as math;

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
import '../../shared/profile/user_cache_provider.dart';
import '../../shared/profile/user_profile.dart';
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
import 'jump_to_latest_button.dart';
import 'jump_to_latest_switcher.dart';
import '../profile/user_profile_sheet.dart';
import 'message_actions.dart';
import 'message_long_press_region.dart';
import 'message_content.dart';
import 'reaction_row.dart';
import '../../shared/read_state/read_state_format.dart';
import '../../shared/read_state/read_state_provider.dart';
import 'send_message_provider.dart';
import 'small_avatar.dart';
import 'sticky_date_header.dart';
import 'timeline_message.dart';

part 'thread_detail_page/nested_thread_summary_row.dart';
part 'thread_detail_page/message_list.dart';
part 'thread_detail_page/sticky_date.dart';
part 'thread_detail_helpers.dart';
part 'thread_detail_page/tail_alignment.dart';
part 'thread_detail_page/thread_message.dart';
part 'thread_detail_page/avatar.dart';

const _landingHighlightDuration = Duration(seconds: 3);
const _landingHighlightDelay = Duration(milliseconds: 50);
const _landingHighlightTransitionDuration = Duration(milliseconds: 300);
const _landingHighlightOpacity = 0.12;

// Keep the direct-position correction finite in case the viewport cannot
// expose its tail (for example, continuously changing media dimensions).
const _latestTailCorrectionLimit = 8;

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
    final relayRepliesAvailable = relayReplyState.value != null;
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
    final hasFetchedReplies = fetchedReplies != null;
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
    final liveHead =
        allMsgs.where((m) => m.id == threadHead.id).firstOrNull ?? threadHead;
    final itemScrollController = useMemoized(ItemScrollController.new);
    final itemPositionsListener = useMemoized(ItemPositionsListener.create);
    final listViewport = useMemoized(LaidOutViewport.new);
    useEffect(() => listViewport.dispose, [listViewport]);
    final stickyDateHeaderState = useValueNotifier(
      StickyDateHeaderState.hidden,
    );
    final stickyDayTimestamp = useValueNotifier<int?>(null);
    final didJumpToInitialMessage = useRef(false);
    final initialHighlightTargetIndex = useState<int?>(null);
    final initialViewportReady = useState(false);
    final followsThreadTail = useRef(false);
    final userOptedOutOfTailFollow = useRef(false);
    final userDragDetachedTailFollow = useRef(false);
    final tailIntent = useMemoized(_ThreadTailIntent.new);
    final initialTailSettle = useMemoized(InitialThreadTailSettle.new);
    final isAtThreadTail = useState(true);
    final tailCorrectionInProgress = useRef(false);
    final tailCorrectionGeneration = useRef(0);
    final activeThreadScrollPosition = useRef<ScrollPosition?>(null);
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
    // Keep the route snapshot usable while the relay query is pending. Once
    // authoritative replies arrive, suppress only the frame(s) used to place
    // the hydrated target, then reveal the settled viewport.
    final threadViewportVisible =
        !relayRepliesAvailable || initialViewportReady.value;

    // Item 0 is the thread head; reply `i` lives at `i + 1`.
    const headIndex = 0;
    int indexForReply(int chronologicalIndex) => chronologicalIndex + 1;
    final tailAnchorIndex = replies.length + 1;
    final stickyDateIndex = _ThreadStickyDateIndex(
      head: liveHead,
      replies: replies,
    );

    void updateStickyDateHeader(Iterable<ItemPosition> positions) {
      final update = stickyDateIndex.resolve(
        positions: positions,
        viewportHeight: viewportHeight,
        stickyTop: frostedAppBarHeight(context) + Grid.twelve,
        stickyHeaderHeight: StickyDateHeader.heightOf(context),
      );
      stickyDateHeaderState.value = update.state;
      stickyDayTimestamp.value = update.activeDayTimestamp;
    }

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

    Widget trackActiveScrollPosition(Widget child) {
      return Builder(
        builder: (itemContext) {
          activeThreadScrollPosition.value = Scrollable.of(
            itemContext,
          ).position;
          return child;
        },
      );
    }

    bool jumpActiveScrollPositionToTail() {
      final position = activeThreadScrollPosition.value;
      if (position == null || !position.hasContentDimensions) return false;
      // Move the one active viewport to its exact end. Unlike indexed
      // jumpTo/scrollTo, this does not reset or cross-fade through a second
      // list, so iOS never exposes the intermediate top-of-thread frame.
      position.jumpTo(position.maxScrollExtent);
      return true;
    }

    Future<bool> animateActiveScrollPositionToTail() async {
      final position = activeThreadScrollPosition.value;
      if (position == null || !position.hasContentDimensions) return false;
      if (MediaQuery.disableAnimationsOf(context)) {
        position.jumpTo(position.maxScrollExtent);
        return true;
      }
      // Match the channel's visible Latest glide while moving only the active
      // thread viewport. The indexed-list animation path can create a temporary
      // second list for distant targets, which caused the old top-frame bounce.
      await position.animateTo(
        position.maxScrollExtent,
        duration: jumpToLatestScrollDuration,
        curve: jumpToLatestScrollCurve,
      );
      return true;
    }

    void finishThreadTailCorrection({
      required bool revealViewport,
      required int generation,
      int corrections = 0,
    }) {
      if (!context.mounted) return;
      if (generation != tailCorrectionGeneration.value) return;
      // A finger drag interrupts the correction. Do not take control back
      // from the user with another jump after they detach from the tail.
      if (!tailCorrectionInProgress.value ||
          tailIntent.isDragging ||
          userOptedOutOfTailFollow.value) {
        tailCorrectionInProgress.value = false;
        isAtThreadTail.value = threadTailIsVisible();
        return;
      }
      final reachedTail = threadTailIsVisible();
      // Lazy children can revise maxScrollExtent for several frames. Keep
      // moving the same active position until the measured tail is visible;
      // the cap only guards pathological layouts that never stabilize.
      if (!reachedTail && corrections < _latestTailCorrectionLimit) {
        jumpActiveScrollPositionToTail();
        WidgetsBinding.instance.addPostFrameCallback(
          (_) => finishThreadTailCorrection(
            revealViewport: revealViewport,
            generation: generation,
            corrections: corrections + 1,
          ),
        );
        WidgetsBinding.instance.scheduleFrame();
        return;
      }
      tailCorrectionInProgress.value = false;
      if (revealViewport) initialViewportReady.value = true;
      isAtThreadTail.value = reachedTail;
    }

    void correctThreadTailInstantly() {
      if (!jumpActiveScrollPositionToTail()) return;
      tailCorrectionInProgress.value = true;
      final generation = ++tailCorrectionGeneration.value;
      isAtThreadTail.value = threadTailIsVisible();
      WidgetsBinding.instance.addPostFrameCallback(
        (_) => finishThreadTailCorrection(
          revealViewport: false,
          generation: generation,
        ),
      );
      WidgetsBinding.instance.scheduleFrame();
    }

    void followThreadTailFromComposer() {
      if (userDragDetachedTailFollow.value) return;
      initialTailSettle.abandon();
      initialViewportReady.value = true;
      tailIntent.endDrag();
      tailIntent.detach();
      userOptedOutOfTailFollow.value = false;
      followsThreadTail.value = true;
      final reachedTail = threadTailIsVisible();
      isAtThreadTail.value = reachedTail;
      if (!reachedTail) correctThreadTailInstantly();
    }

    useEffect(
      () {
        void onPositionsChanged() {
          updateStickyDateHeader(itemPositionsListener.itemPositions.value);
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
      },
      [
        itemPositionsListener,
        replies.length,
        liveHead.createdAt,
        viewportHeight,
      ],
    );

    useEffect(() {
      stickyDateHeaderState.value = StickyDateHeaderState.hidden;
      stickyDayTimestamp.value = null;
      return null;
    }, [threadHead.id]);

    void scrollToThreadLatest() {
      if (!itemScrollController.isAttached) return;
      initialTailSettle.abandon();
      tailIntent.endDrag();
      tailIntent.detach();
      // An explicit Latest tap supersedes a still-pending Inbox/Activity
      // deep-link. Without consuming that one-shot intent, the authoritative
      // thread query can finish after this navigation and jump back to the
      // originally linked older message.
      didJumpToInitialMessage.value = true;
      initialHighlightTargetIndex.value = null;
      initialTargetReadyForHighlight.value = false;
      userOptedOutOfTailFollow.value = false;
      userDragDetachedTailFollow.value = false;
      followsThreadTail.value = true;
      tailCorrectionInProgress.value = true;
      final generation = ++tailCorrectionGeneration.value;

      Future<void> navigateToTail() async {
        if (!await animateActiveScrollPositionToTail()) {
          if (generation == tailCorrectionGeneration.value) {
            tailCorrectionInProgress.value = false;
          }
          return;
        }
        finishThreadTailCorrection(
          revealViewport: true,
          generation: generation,
        );
      }

      unawaited(navigateToTail());
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
        if (targetIndex == null) {
          initialTailSettle.abandon();
          initialViewportReady.value = true;
          return null;
        }
        if (didJumpToInitialMessage.value) return null;
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
            if (!context.mounted) return;
            initialTargetReadyForHighlight.value = true;
            initialViewportReady.value = true;
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
          onSettled: () {
            if (context.mounted) initialViewportReady.value = true;
          },
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
                child: _ThreadMessageList(
                  viewport: listViewport,
                  onUserScrollStart: () {
                    initialTailSettle.abandon();
                    initialViewportReady.value = true;
                    tailCorrectionInProgress.value = false;
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
                  visible: threadViewportVisible,
                  itemScrollController: itemScrollController,
                  itemPositionsListener: itemPositionsListener,
                  bottomInset: timelineBottomInset,
                  replies: replies,
                  trackActiveScrollPosition: trackActiveScrollPosition,
                  headIsDeleted: liveDeletionHidesHead,
                  head: liveHead,
                  stickyDayTimestamp: stickyDayTimestamp,
                  channelNames: channelNamesMap,
                  channelId: channelId,
                  currentPubkey: currentPubkey,
                  highlightedMessageId: highlightedMessageId.value,
                  allMessages: allMsgs,
                  isMember: isMember,
                  isArchived: isArchived,
                  composerFocusNode: composerFocusNode,
                  restoreComposerFocus: () =>
                      restoreComposerFocus.value?.call(),
                  childrenByParent: childrenByParent,
                ),
              ),
              if (!isMember || isArchived)
                _ThreadTypingIndicator(entries: threadTyping, animated: false),
            ],
          ),
          if (threadViewportVisible)
            Positioned(
              left: 0,
              right: 0,
              top: frostedAppBarHeight(context) + Grid.twelve,
              child: StickyDateHeader(
                key: const ValueKey('thread-sticky-date-header'),
                state: stickyDateHeaderState,
              ),
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
          Positioned(
            left: 0,
            right: 0,
            bottom: navigationBottomInset + Grid.xs,
            child: Center(
              child: JumpToLatestSwitcher(
                id: 'thread',
                visible:
                    threadViewportVisible &&
                    hasFetchedReplies &&
                    !isAtThreadTail.value,
                onPressed: scrollToThreadLatest,
              ),
            ),
          ),
        ],
      ),
    );
  }
}
