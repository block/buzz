part of '../thread_detail_page.dart';

class _ThreadMessageList extends StatelessWidget {
  final LaidOutViewport viewport;
  final VoidCallback onUserScrollStart;
  final VoidCallback onUserScrollEnd;
  final bool visible;
  final ItemScrollController itemScrollController;
  final ItemPositionsListener itemPositionsListener;
  final double bottomInset;
  final List<TimelineMessage> replies;
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
    required this.viewport,
    required this.onUserScrollStart,
    required this.onUserScrollEnd,
    required this.visible,
    required this.itemScrollController,
    required this.itemPositionsListener,
    required this.bottomInset,
    required this.replies,
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

  @override
  Widget build(BuildContext context) {
    const headIndex = 0;
    final tailAnchorIndex = replies.length + 1;

    return LaidOutViewportReporter(
      viewport: viewport,
      child: KeyboardDismissOnDrag(
        onUserScrollStart: onUserScrollStart,
        onUserScrollEnd: onUserScrollEnd,
        child: Opacity(
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
                  if (headIsDeleted) {
                    return trackActiveScrollPosition(
                      const Padding(
                        key: ValueKey('thread-message-deleted'),
                        padding: EdgeInsets.only(bottom: Grid.xs),
                        child: Text('This message was deleted'),
                      ),
                    );
                  }
                  return trackActiveScrollPosition(
                    Padding(
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
                            padding: const EdgeInsets.symmetric(
                              vertical: Grid.xxs,
                            ),
                            child: Row(
                              children: [
                                Text(
                                  '${replies.length} ${replies.length == 1 ? 'reply' : 'replies'}',
                                  style: context.textTheme.labelMedium
                                      ?.copyWith(
                                        color: context.colors.onSurfaceVariant,
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
                    ),
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
                  Padding(
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
                          isHighlighted: reply.id == highlightedMessageId,
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
                );
              },
            ),
          ),
        ),
      ),
    );
  }
}
