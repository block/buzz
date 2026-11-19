import '../../shared/agents/is_quiet_agent_message.dart';
import 'date_formatters.dart';
import 'timeline_message.dart';

/// Activity is identified by its signed audience, never by the author's role.
bool isInlineActivity(TimelineMessage message) =>
    !message.isSystem && isQuietAgentMessage(message.tags);

/// Consecutive quiet updates share a disclosure within one conversation/day.
/// Public replies and system notices always remain separate timeline rows.
List<List<T>> groupInlineActivity<T>(
  List<T> items,
  TimelineMessage Function(T) messageOf, {
  String? revealMessageId,
}) {
  final groups = <List<T>>[];
  for (final item in items) {
    final message = messageOf(item);
    final previous = groups.isEmpty ? null : messageOf(groups.last.last);
    if (previous != null &&
        previous.id != revealMessageId &&
        message.id != revealMessageId &&
        isInlineActivity(previous) &&
        isInlineActivity(message) &&
        previous.rootId == message.rootId &&
        isSameDay(previous.createdAt, message.createdAt)) {
      groups.last.add(item);
    } else {
      groups.add([item]);
    }
  }
  return groups;
}

/// Preserve membership grouping while folding activity in the main timeline.
List<List<MainTimelineEntry>> groupConversationEntries(
  List<MainTimelineEntry> entries, {
  String? revealMessageId,
}) => groupInlineActivity(
  groupMembershipTimelineEntries(entries),
  (group) => group.first.message,
  revealMessageId: revealMessageId,
).map((groups) => groups.expand((group) => group).toList()).toList();
