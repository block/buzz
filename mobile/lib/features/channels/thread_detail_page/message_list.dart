part of '../thread_detail_page.dart';

class _ThreadMessageList extends HookWidget {
  final double headHeight;
  final ValueChanged<double> onHeadHeightChanged;
  final LaidOutViewport viewport;
  final VoidCallback onUserScrollStart;
  final VoidCallback onUserScrollEnd;
  final bool visible;
  final ItemScrollController itemScrollController;
  final ItemPositionsListener itemPositionsListener;
  final double bottomInset;
  final List<TimelineMessage> replies;
  final AsyncValue<List<NostrEvent>> relayReplyState;
  final VoidCallback onRetryReplies;
  final Map<String, DateTime> localSendAnimations;
  final Widget Function(Widget child) trackActiveScrollPosition;
  final bool headIsDeleted;
  final TimelineMessage head;
  final ValueNotifier<int?> stickyDayTimestamp;
  final Map<String, String> channelNames;
  final String channelId;
  final String? currentPubkey;
  final String? highlightedMessageId;
  final List<TimelineMessage> allMessages;
  final bool isMember;
  final bool isArchived;
  final FocusNode composerFocusNode;
  final VoidCallback restoreComposerFocus;
  final Map<String, List<TimelineMessage>> childrenByParent;

  const _ThreadMessageList({
    required this.headHeight,
    required this.onHeadHeightChanged,
    required this.viewport,
    required this.onUserScrollStart,
    required this.onUserScrollEnd,
    required this.visible,
    required this.itemScrollController,
    required this.itemPositionsListener,
    required this.bottomInset,
    required this.replies,
    required this.relayReplyState,
    required this.onRetryReplies,
    required this.localSendAnimations,
    required this.trackActiveScrollPosition,
    required this.headIsDeleted,
    required this.head,
    required this.stickyDayTimestamp,
    required this.channelNames,
    required this.channelId,
    required this.currentPubkey,
    required this.highlightedMessageId,
    required this.allMessages,
    required this.isMember,
    required this.isArchived,
    required this.composerFocusNode,
    required this.restoreComposerFocus,
    required this.childrenByParent,
  });

  String get _replySummary {
    if (!relayReplyState.hasValue && replies.isEmpty) {
      return relayReplyState.isLoading
          ? 'Loading replies…'
          : 'Couldn’t load replies';
    }
    final count =
        '${replies.length} ${replies.length == 1 ? 'reply' : 'replies'}';
    if (!relayReplyState.hasError) return count;
    return '$count · ${relayReplyState.isLoading ? 'Retrying…' : 'Couldn’t refresh'}';
  }

  Widget _buildHead(BuildContext context) => _ThreadHeadLayout(
    onHeightChanged: onHeadHeightChanged,
    child: _buildHeadContent(context),
  );

  Widget _buildHeadContent(BuildContext context) {
    if (headIsDeleted) {
      return const Padding(
        key: ValueKey('thread-message-deleted'),
        padding: EdgeInsets.only(bottom: Grid.xs),
        child: Text('This message was deleted'),
      );
    }
    return Padding(
      key: ValueKey('thread-message-group-${head.id}'),
      padding: const EdgeInsets.only(bottom: Grid.xs),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          DayDivider(
            label: formatDayHeading(head.createdAt),
            dayTimestamp: head.createdAt,
            stickyDayTimestamp: stickyDayTimestamp,
          ),
          _ThreadMessage(
            message: head,
            channelNames: channelNames,
            channelId: channelId,
            currentPubkey: currentPubkey,
            showAuthor: true,
            isHighlighted: head.id == highlightedMessageId,
            allMessages: allMessages,
            isMember: isMember,
            isArchived: isArchived,
            isThreadHead: true,
            composerFocusNode: composerFocusNode,
            restoreComposerFocus: restoreComposerFocus,
          ),
          Padding(
            padding: const EdgeInsets.symmetric(vertical: Grid.xxs),
            child: Row(
              children: [
                Flexible(
                  child: Semantics(
                    liveRegion: true,
                    child: Text(
                      _replySummary,
                      style: context.textTheme.labelMedium?.copyWith(
                        color: context.colors.onSurfaceVariant,
                        fontWeight: FontWeight.w600,
                      ),
                    ),
                  ),
                ),
                if (relayReplyState.hasError && !relayReplyState.isLoading)
                  IconButton(
                    key: const ValueKey('thread-replies-retry'),
                    onPressed: onRetryReplies,
                    tooltip: 'Retry',
                    icon: const Icon(BuzzIcons.refreshCcw, size: 16),
                  ),
                const SizedBox(width: Grid.xxs),
                Expanded(child: Divider(color: context.colors.outlineVariant)),
              ],
            ),
          ),
        ],
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    final headLink = useMemoized(LayerLink.new);
    final loadingHeadLink = useMemoized(LayerLink.new);
    final headScrollPosition = useRef<ScrollPosition?>(null);
    const headIndex = 0;
    final tailAnchorIndex = replies.length + 1;

    return LaidOutViewportReporter(
      viewport: viewport,
      child: KeyboardDismissOnDrag(
        onUserScrollStart: onUserScrollStart,
        onUserScrollEnd: onUserScrollEnd,
        child: ClipRect(
          child: LayoutBuilder(
            builder: (context, constraints) => Stack(
              fit: StackFit.expand,
              children: [
                Opacity(
                  key: const ValueKey('thread-initial-viewport-gate'),
                  opacity: visible ? 1 : 0,
                  child: IgnorePointer(
                    ignoring: !visible,
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
                        bottom: Grid.xs + bottomInset,
                      ),
                      // Head + replies + a stable zero-content tail target. The
                      // anchor lets Latest align the end directly rather than
                      // asking the final reply's leading edge to overshoot the
                      // viewport and rebound against the scroll extent.
                      itemCount: replies.length + 2,
                      itemBuilder: (context, index) {
                        if (index == tailAnchorIndex) {
                          return trackActiveScrollPosition(
                            const SizedBox(
                              key: ValueKey('thread-tail-anchor'),
                              height: 1,
                            ),
                          );
                        }
                        if (index == headIndex) {
                          return Builder(
                            builder: (headContext) {
                              headScrollPosition.value = Scrollable.of(
                                headContext,
                              ).position;
                              return trackActiveScrollPosition(
                                CompositedTransformTarget(
                                  link: headLink,
                                  child: SizedBox(height: headHeight),
                                ),
                              );
                            },
                          );
                        }

                        // Chronological list: index 1 = oldest reply.
                        final chronologicalIndex = index - 1;
                        final reply = replies[chronologicalIndex];
                        final previousReply = chronologicalIndex > 0
                            ? replies[chronologicalIndex - 1]
                            : null;
                        final previousMessage = previousReply ?? head;
                        final showDayDivider = !isSameDay(
                          previousMessage.createdAt,
                          reply.createdAt,
                        );
                        final showAuthor =
                            previousReply == null ||
                            showDayDivider ||
                            previousReply.pubkey.toLowerCase() !=
                                reply.pubkey.toLowerCase() ||
                            (reply.createdAt - previousReply.createdAt) > 300;

                        // Check if this reply itself has children (nested thread).
                        final nestedChildren = childrenByParent[reply.id];
                        final nestedSummary =
                            nestedChildren != null && nestedChildren.isNotEmpty
                            ? _buildNestedSummary(reply.id, nestedChildren)
                            : null;

                        return trackActiveScrollPosition(
                          LocalMessageSendTransition(
                            key: ValueKey('thread-message-send-${reply.id}'),
                            animate: isRecentLocalMessageSendAnimation(
                              localSendAnimations,
                              reply.id,
                            ),
                            startOffsetFactor: showAuthor
                                ? localMessageSendTransitionAvatarStartOffset
                                : localMessageSendTransitionStartOffset,
                            child: Padding(
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
                                      dayTimestamp: reply.createdAt,
                                      stickyDayTimestamp: stickyDayTimestamp,
                                    ),
                                  _ThreadMessage(
                                    message: reply,
                                    channelNames: channelNames,
                                    channelId: channelId,
                                    currentPubkey: currentPubkey,
                                    showAuthor: showAuthor,
                                    isHighlighted:
                                        reply.id == highlightedMessageId,
                                    allMessages: allMessages,
                                    isMember: isMember,
                                    isArchived: isArchived,
                                    composerFocusNode: composerFocusNode,
                                    restoreComposerFocus: restoreComposerFocus,
                                  ),
                                  if (nestedSummary != null)
                                    _NestedThreadSummaryRow(
                                      summary: nestedSummary,
                                      replyMessage: reply,
                                      allMessages: allMessages,
                                      channelId: channelId,
                                      currentPubkey: currentPubkey,
                                      isMember: isMember,
                                      isArchived: isArchived,
                                    ),
                                ],
                              ),
                            ),
                          ),
                        );
                      },
                    ),
                  ),
                ),
                // Keep one live head in a stable subtree. Only its paint position
                // follows the list, so media and reaction ownership never transfer.
                Positioned(
                  left: Grid.gutter,
                  right: Grid.gutter,
                  top: frostedAppBarHeight(context),
                  child: CompositedTransformTarget(
                    link: loadingHeadLink,
                    child: const SizedBox(),
                  ),
                ),
                Positioned(
                  left: Grid.gutter,
                  right: Grid.gutter,
                  top: 0,
                  child: CompositedTransformFollower(
                    link: visible ? headLink : loadingHeadLink,
                    showWhenUnlinked: false,
                    child: _ThreadHeadScrollInput(
                      enabled: visible,
                      onUserScrollStart: onUserScrollStart,
                      onUserScrollEnd: onUserScrollEnd,
                      scrollPosition: () => headScrollPosition.value,
                      positions: itemPositionsListener.itemPositions,
                      viewportTopEdge:
                          frostedAppBarHeight(context) / constraints.maxHeight,
                      child: ConstrainedBox(
                        constraints: BoxConstraints(
                          maxHeight: visible
                              ? double.infinity
                              : math.max(
                                  0,
                                  constraints.maxHeight -
                                      frostedAppBarHeight(context) -
                                      Grid.xs -
                                      bottomInset,
                                ),
                        ),
                        child: SingleChildScrollView(
                          key: const ValueKey('thread-initial-head'),
                          primary: false,
                          physics: visible
                              ? const NeverScrollableScrollPhysics()
                              : null,
                          child: _buildHead(context),
                        ),
                      ),
                    ),
                  ),
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}
