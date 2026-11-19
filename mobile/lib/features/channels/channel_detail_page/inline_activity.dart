part of '../channel_detail_page.dart';

extension on _MessageList {
  Widget _buildActivityRow(
    BuildContext context,
    List<MainTimelineEntry> entries,
    Map<String, String> names, {
    required bool showDayDivider,
    required bool inheritLiveExpansion,
    required List<TypingEntry> typing,
    required VoidCallback onToggle,
    required ValueNotifier<int?> stickyDayTimestamp,
  }) {
    final messages = entries.map((entry) => entry.message).toList();
    final authors = messages
        .map((message) => message.pubkey.toLowerCase())
        .toSet();
    final remainingTyping = typing
        .where((entry) => !authors.contains(entry.pubkey.toLowerCase()))
        .toList();
    return Column(
      key: ValueKey('activity-row-${messages.first.id}'),
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
          revealMessageId: initialMessageId,
          onToggle: onToggle,
          messageBuilder: (message) {
            final entry = entries.firstWhere(
              (entry) => entry.message.id == message.id,
            );
            final working = threadWorking[message.id.toLowerCase()];
            return _MessageBubble(
              message: message,
              showAuthor: true,
              hasReplies: entry.summary != null,
              replyFooter: entry.summary == null && working == null
                  ? null
                  : _ThreadSummaryRow(
                      summary: entry.summary,
                      working: working ?? const [],
                      message: message,
                      allMessages: allMessages,
                      channelId: channelId,
                      currentPubkey: currentPubkey,
                      isMember: isMember,
                      isArchived: isArchived,
                    ),
              channelNames: names,
              currentChannelId: channelId,
              currentPubkey: currentPubkey,
              allMessages: allMessages,
              isMember: isMember,
              isArchived: isArchived,
              composerFocusNode: composerFocusNode,
              restoreComposerFocus: restoreComposerFocus,
            );
          },
        ),
        if (remainingTyping.isNotEmpty)
          InlineAgentTyping(channelId: channelId, entries: remainingTyping),
      ],
    );
  }
}
