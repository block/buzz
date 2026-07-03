part of '../compose_bar.dart';

const _typingThrottleMs = 3000;

/// Cap on ranked mention suggestions shown — matches desktop's
/// `MENTION_SUGGESTION_LIMIT`.
const _mentionSuggestionLimit = 50;

/// Walk backward from [cursor] looking for [trigger] (e.g. `@` or `#`) at a
/// word boundary. Returns the index of the trigger character, or `null` if none
/// is found.
///
/// When [stopAtSpace] is `true` the walk stops at both spaces and newlines —
/// appropriate for `#channel` names which are kebab-case slugs without spaces.
/// When `false`, only newlines stop the walk, allowing multi-word queries like
/// `@Alice Smith` to match members with multi-word display names.
@visibleForTesting
int? findTrigger(
  String text,
  int cursor,
  String trigger, {
  bool stopAtSpace = true,
}) {
  for (var i = cursor - 1; i >= 0; i--) {
    final ch = text[i];
    if (ch == '\n') break;
    if (stopAtSpace && ch == ' ') break;
    if (ch == trigger) {
      if (i == 0 || text[i - 1] == ' ' || text[i - 1] == '\n') {
        return i;
      }
      break;
    }
  }
  return null;
}

/// Replace the range `[start, cursor)` with [replacement] and move the cursor
/// to the end of the replacement. Used by both mention and channel insertion.
@visibleForTesting
void spliceAndMoveCursor(
  TextEditingController controller,
  FocusNode focusNode, {
  required int start,
  required String replacement,
}) {
  final text = controller.text;
  final cursor =
      (controller.selection.isValid
              ? controller.selection.baseOffset
              : text.length)
          .clamp(start, text.length);

  final before = text.substring(0, start);
  final after = text.substring(cursor);
  controller.text = '$before$replacement$after';
  controller.selection = TextSelection.collapsed(
    offset: start + replacement.length,
  );
  focusNode.requestFocus();
}

/// Insert [trigger] (e.g. `@` or `#`) at the cursor position, prefixed with
/// a space if needed for word separation. Used by `triggerMention` and
/// `triggerChannel`.
void _insertTriggerAtCursor(
  TextEditingController controller,
  FocusNode focusNode,
  String trigger,
) {
  final text = controller.text;
  final cursor = controller.selection.isValid
      ? controller.selection.baseOffset
      : text.length;
  final needsSpace =
      cursor > 0 && text[cursor - 1] != ' ' && text[cursor - 1] != '\n';
  final insert = needsSpace ? ' $trigger' : trigger;
  final before = text.substring(0, cursor);
  final after = text.substring(cursor);
  controller.text = '$before$insert$after';
  controller.selection = TextSelection.collapsed(
    offset: cursor + insert.length,
  );
  focusNode.requestFocus();
}

/// Send a typing indicator over the WebSocket (fire-and-forget).
///
/// Desktop sends these as `["EVENT", signedEvent]` over the WebSocket — not
/// via HTTP. Ephemeral events like typing indicators are broadcast-only and
/// the relay doesn't persist them, so the HTTP `/api/events` endpoint may
/// silently discard them.
void _sendTypingIndicator(
  WidgetRef ref, {
  required String channelId,
  String? threadHeadId,
  String? rootId,
}) {
  try {
    final config = ref.read(relayConfigProvider);
    final nsec = config.nsec;
    if (nsec == null || nsec.isEmpty) return;

    final privkeyHex = nostr.Nip19.decode(payload: nsec).data;
    if (privkeyHex.isEmpty) return;

    final tags = <List<String>>[
      ['h', channelId],
      if (threadHeadId != null && rootId != null && rootId != threadHeadId) ...[
        ['e', rootId, '', 'root'],
        ['e', threadHeadId, '', 'reply'],
      ] else if (threadHeadId != null)
        ['e', threadHeadId, '', 'reply'],
    ];

    final event = nostr.Event.from(
      kind: EventKind.typingIndicator,
      content: '',
      tags: tags,
      secretKey: privkeyHex,
      verify: false,
    );

    // Send directly over WebSocket — fire-and-forget, matching desktop.
    final session = ref.read(relaySessionProvider.notifier);
    session.sendRaw(['EVENT', event.toMap()]);
  } catch (_) {
    // Fire-and-forget — typing indicator failure is non-fatal.
  }
}
