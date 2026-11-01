import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'agent_typing_bubble.dart';
import 'channel_typing_provider.dart';
import 'timeline_message.dart';

/// Keeps completion handoffs scoped to newly arriving replies, never history.
({Map<String, int> messageIds, List<TypingEntry> typing}) useAgentReplyHandoffs(
  String channelId,
  List<TypingEntry> typing,
  List<TimelineMessage> messages,
) {
  final tracker = useMemoized(_ReplyHandoffs.new, [channelId]);
  return tracker.update(typing, messages);
}

/// Limits the handoff to a fresh arrival, avoiding replay when scrolling back.
bool isRecentAgentReplyHandoff(Map<String, int> entries, String id) =>
    (entries[id] ?? 0) > DateTime.now().millisecondsSinceEpoch;

class _ReplyHandoffs {
  Set<String>? seen;
  final pending = <(String, String?), TypingEntry>{};
  final completed = <(String, String?), int>{};
  final handoffs = <String, int>{};

  ({Map<String, int> messageIds, List<TypingEntry> typing}) update(
    List<TypingEntry> typing,
    List<TimelineMessage> messages,
  ) {
    final now = DateTime.now().millisecondsSinceEpoch;
    pending.removeWhere((_, entry) => entry.expiresAtMs <= now);
    completed.removeWhere((_, at) => at + 8000 <= now);
    for (final entry in typing) {
      final scope = (entry.pubkey.toLowerCase(), entry.threadHeadId);
      if ((completed[scope] ?? -1) < entry.expiresAtMs - 8000) {
        pending[scope] = entry;
      }
    }
    final ids = messages.map((message) => message.id).toSet();
    handoffs.removeWhere((id, expires) => !ids.contains(id) || expires <= now);
    if (seen != null) {
      for (final message in messages) {
        if (seen!.contains(message.id) || message.isSystem) continue;
        final scope = (
          message.pubkey.toLowerCase(),
          message.rootId ?? message.parentId,
        );
        final activity = pending[scope];
        if (activity != null &&
            message.createdAt * 1000 >= activity.expiresAtMs - 8000) {
          handoffs[message.id] = now + 1000;
          completed[scope] = message.createdAt * 1000;
          pending.remove(scope);
        }
      }
    }
    seen = ids;
    return (
      messageIds: handoffs,
      typing: [
        for (final entry in typing)
          if ((completed[(entry.pubkey.toLowerCase(), entry.threadHeadId)] ??
                  -1) <
              entry.expiresAtMs - 8000)
            entry,
      ],
    );
  }
}

/// Grows the pending bubble into the real message using its final content size.
class AgentReplyHandoff extends HookWidget {
  const AgentReplyHandoff({
    super.key,
    required this.animate,
    required this.child,
  });
  final bool animate;
  final Widget child;

  @override
  Widget build(BuildContext context) {
    final reduced = MediaQuery.disableAnimationsOf(context);
    final revealed = useState(!animate || reduced);
    useEffect(() {
      if (!revealed.value) {
        WidgetsBinding.instance.addPostFrameCallback((_) {
          if (context.mounted) revealed.value = true;
        });
      }
      return null;
    }, const []);
    if (!animate || reduced) return child;
    return AnimatedSize(
      key: const ValueKey('agent-reply-handoff'),
      duration: const Duration(milliseconds: 220),
      curve: const Cubic(0.23, 1, 0.32, 1),
      alignment: Alignment.topLeft,
      child: revealed.value ? child : const AgentTypingBubble(),
    );
  }
}
