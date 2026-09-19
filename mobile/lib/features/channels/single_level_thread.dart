import 'package:flutter/widgets.dart';
import 'timeline_message.dart';

/// Include historical descendants even when intermediate parents are absent.
List<TimelineMessage> flatThreadReplies(
  List<TimelineMessage> messages,
  String rootId,
) {
  final byParent = <String, List<TimelineMessage>>{};
  final included = <String, TimelineMessage>{};
  final pending = <String>[rootId];
  final visited = <String>{};
  for (final message in messages) {
    final parentId = message.parentId;
    if (parentId != null) byParent.putIfAbsent(parentId, () => []).add(message);
    if (message.id != rootId && message.rootId == rootId) {
      included[message.id] = message;
      pending.add(message.id);
    }
  }
  while (pending.isNotEmpty) {
    final parentId = pending.removeLast();
    if (!visited.add(parentId)) continue;
    for (final child in byParent[parentId] ?? const <TimelineMessage>[]) {
      if (child.id == rootId) continue;
      included[child.id] = child;
      pending.add(child.id);
    }
  }
  return included.values.toList()..sort((a, b) {
    final time = a.createdAt.compareTo(b.createdAt);
    return time != 0 ? time : a.id.compareTo(b.id);
  });
}

String? replyContextId(TimelineMessage message) {
  for (final tag in message.tags) {
    if (tag.length == 2 && tag[0] == 'reply-context') return tag[1];
  }
  return message.parentId != message.rootId ? message.parentId : null;
}

/// Reply actions inside a thread reuse its composer and navigation surface.
class ThreadReplyScope extends InheritedWidget {
  final ValueChanged<TimelineMessage> onReply;
  final ValueChanged<String> onReveal;
  const ThreadReplyScope({
    super.key,
    required this.onReply,
    required this.onReveal,
    required super.child,
  });
  static ThreadReplyScope? maybeOf(BuildContext context) =>
      context.getInheritedWidgetOfExactType<ThreadReplyScope>();
  @override
  bool updateShouldNotify(ThreadReplyScope oldWidget) =>
      onReply != oldWidget.onReply || onReveal != oldWidget.onReveal;
}
