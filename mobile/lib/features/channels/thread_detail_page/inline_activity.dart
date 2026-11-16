part of '../thread_detail_page.dart';

extension on _ThreadMessageList {
  Widget _buildActivityRow(
    List<TimelineMessage> messages, {
    required bool showDayDivider,
    required bool inheritLiveExpansion,
    required List<TypingEntry> typing,
  }) {
    final authors = messages
        .map((message) => message.pubkey.toLowerCase())
        .toSet();
    final remainingTyping = typing
        .where((entry) => !authors.contains(entry.pubkey.toLowerCase()))
        .toList();
    return Column(
      key: ValueKey('thread-activity-row-${messages.first.id}'),
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        if (showDayDivider)
          DayDivider(
            label: formatDayHeading(messages.first.createdAt),
            dayTimestamp: messages.first.createdAt,
            stickyDayTimestamp: stickyDayTimestamp,
            scrollVisible: dateScrollActivity,
          ),
        InlineAgentActivity(
          key: ValueKey('activity-${messages.first.id}'),
          channelId: channelId,
          inheritLiveExpansion: inheritLiveExpansion,
          messages: messages,
          liveEntries: typing
              .where((entry) => authors.contains(entry.pubkey.toLowerCase()))
              .toList(),
          working: typing.any(
            (entry) => authors.contains(entry.pubkey.toLowerCase()),
          ),
          revealMessageId: revealMessageId,
          onToggle: () {
            onUserScrollStart();
            WidgetsBinding.instance.addPostFrameCallback(
              (_) => onUserScrollEnd(),
            );
          },
          messageBuilder: (message) {
            final children = childrenByParent[message.id];
            return Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                _ThreadMessage(
                  message: message,
                  channelNames: channelNames,
                  channelId: channelId,
                  currentPubkey: currentPubkey,
                  showAuthor: true,
                  isHighlighted: message.id == highlightedMessageId,
                  allMessages: allMessages,
                  isMember: isMember,
                  isArchived: isArchived,
                  composerFocusNode: composerFocusNode,
                  restoreComposerFocus: restoreComposerFocus,
                ),
                if (children != null && children.isNotEmpty)
                  _NestedThreadSummaryRow(
                    summary: _buildNestedSummary(message.id, children),
                    replyMessage: message,
                    allMessages: allMessages,
                    channelId: channelId,
                    currentPubkey: currentPubkey,
                    isMember: isMember,
                    isArchived: isArchived,
                  ),
              ],
            );
          },
        ),
        if (remainingTyping.isNotEmpty)
          InlineAgentTyping(
            channelId: channelId,
            entries: remainingTyping,
            knownAgents: knownAgents,
          ),
      ],
    );
  }
}
