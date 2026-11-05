part of '../channel_detail_page.dart';

Widget? _buildThreadReplyFooter(_MessageList list, MainTimelineEntry entry) {
  final message = entry.message;
  return entry.summary == null &&
          !list.threadWorking.containsKey(message.id.toLowerCase())
      ? null
      : _ThreadSummaryRow(
          summary: entry.summary,
          working: list.threadWorking[message.id.toLowerCase()] ?? const [],
          message: message,
          allMessages: list.allMessages,
          channelId: list.channelId,
          currentPubkey: list.currentPubkey,
          isMember: list.isMember,
          canContribute: list.mayContribute,
          isArchived: list.isArchived,
        );
}
