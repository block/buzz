import 'dart:async';

import 'package:flutter/cupertino.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import 'package:buzz/shared/theme/buzz_icons.dart';
import '../../shared/theme/theme.dart';
import 'local_send_status_provider.dart';
import 'send_message_provider.dart';

/// Adds delivery status to an outgoing message bubble.
///
/// Sending dims the bubble and the latest message reads "Sending…", then
/// "Sent". A failed message keeps its bubble with a red alert badge and a
/// "Not Delivered" line; tapping either offers Try Again or Delete.
class LocalSendStatusDecoration extends ConsumerWidget {
  /// Creates status chrome around [child] for the message [eventId].
  const LocalSendStatusDecoration({
    super.key,
    required this.channelId,
    required this.eventId,
    required this.child,
    this.enabled = true,
  });

  /// Channel the message was sent to.
  final String channelId;

  /// Signed event id of the outgoing message.
  final String eventId;

  /// The message bubble.
  final Widget child;

  /// False for other people's messages, which never carry local status.
  final bool enabled;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    if (!enabled) return child;
    final status = ref.watch(
      localSendStatusForProvider((channelId: channelId, eventId: eventId)),
    );
    if (status == null) return child;
    final failed = status == LocalSendStatus.failed;
    final error = context.colors.error;
    final labelStyle = context.textTheme.labelSmall?.copyWith(
      fontSize: 11,
      height: 15 / 11,
      color: failed ? error : context.tertiaryTextColor,
      fontWeight: failed ? FontWeight.w600 : FontWeight.w500,
    );
    final label = switch (status) {
      LocalSendStatus.sending => 'Sending…',
      LocalSendStatus.sent => 'Sent',
      LocalSendStatus.failed => 'Not Delivered',
    };
    void openActions() => unawaited(_showFailedActions(context, ref));
    final bubble = AnimatedOpacity(
      opacity: status == LocalSendStatus.sending ? 0.6 : 1,
      duration: const Duration(milliseconds: 150),
      child: child,
    );
    return Column(
      crossAxisAlignment: CrossAxisAlignment.end,
      mainAxisSize: MainAxisSize.min,
      children: [
        if (failed)
          Row(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.center,
            children: [
              Flexible(child: bubble),
              const SizedBox(width: Grid.half),
              Semantics(
                button: true,
                label: 'Message not delivered. Show options',
                excludeSemantics: true,
                onTap: openActions,
                child: GestureDetector(
                  key: ValueKey('local-send-retry-$eventId'),
                  behavior: HitTestBehavior.opaque,
                  onTap: openActions,
                  child: Padding(
                    padding: const EdgeInsets.all(Grid.quarter),
                    child: Icon(BuzzIcons.circleAlert, size: 22, color: error),
                  ),
                ),
              ),
            ],
          )
        else
          bubble,
        Padding(
          padding: EdgeInsets.only(
            top: Grid.quarter,
            right: failed ? 22 + Grid.half + Grid.half + Grid.xs : Grid.xs,
          ),
          child: GestureDetector(
            behavior: HitTestBehavior.opaque,
            onTap: failed ? openActions : null,
            child: Semantics(
              liveRegion: true,
              excludeSemantics: failed,
              child: Text(
                label,
                key: ValueKey('local-send-status-$eventId'),
                style: labelStyle,
              ),
            ),
          ),
        ),
      ],
    );
  }

  Future<void> _showFailedActions(BuildContext context, WidgetRef ref) async {
    final personal = ref
        .read(localSendStatusProvider(channelId).notifier)
        .isPersonal(eventId);
    // Read before the sheet: the row can rebuild under it and unmount [ref].
    final sender = ref.read(
      personal ? meSendMessageProvider : sendMessageProvider,
    );
    final action = await showCupertinoModalPopup<_FailedAction>(
      context: context,
      builder: (sheetContext) => CupertinoActionSheet(
        message: const Text('Your message was not delivered.'),
        actions: [
          CupertinoActionSheetAction(
            onPressed: () =>
                Navigator.of(sheetContext).pop(_FailedAction.retry),
            child: const Text('Try Again'),
          ),
          CupertinoActionSheetAction(
            isDestructiveAction: true,
            onPressed: () =>
                Navigator.of(sheetContext).pop(_FailedAction.delete),
            child: const Text('Delete Message'),
          ),
        ],
        cancelButton: CupertinoActionSheetAction(
          onPressed: () => Navigator.of(sheetContext).pop(),
          child: const Text('Cancel'),
        ),
      ),
    );
    if (action == null) return;
    switch (action) {
      case _FailedAction.retry:
        await sender.retry(channelId: channelId, eventId: eventId);
      case _FailedAction.delete:
        sender.discard(channelId: channelId, eventId: eventId);
    }
  }
}

enum _FailedAction { retry, delete }
