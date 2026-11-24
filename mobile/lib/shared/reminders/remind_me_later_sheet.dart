import 'dart:async';

import 'package:buzz/shared/widgets/feedback_snackbar.dart';
import 'package:flutter/material.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../theme/theme.dart';
import '../widgets/modal_presentation.dart';
import 'reminder_service.dart';
import 'reminder_time_choices.dart';

/// Bottom sheet for "Remind me later": desktop-parity presets plus a native
/// date/time picker, publishing the same author-private kind-30300 reminder
/// events desktop reads.
///
/// With [existing], the chosen time reschedules that pending reminder (same
/// `d` tag, note kept) instead of adding a second one. [onSaved] runs after
/// the relay accepts the reminder.
void showRemindMeLaterSheet({
  required BuildContext context,
  required WidgetRef ref,
  required ReminderTarget target,
  ({String dTag, String? note})? existing,
  VoidCallback? onSaved,
}) {
  final service = ref.read(reminderServiceProvider);
  final messenger = ScaffoldMessenger.of(context);
  if (service == null) {
    messenger.showFeedbackSnackBar(
      const SnackBar(content: Text('Sign in to set reminders')),
      kind: ToastKind.generic,
    );
    return;
  }

  Future<void> submit(int notBefore) async {
    try {
      await service.createReminder(
        target: target,
        notBefore: notBefore,
        note: existing?.note,
        replacingDTag: existing?.dTag,
      );
      onSaved?.call();
      messenger.showFeedbackSnackBar(
        SnackBar(
          content: Text(existing == null ? 'Reminder set' : 'Reminder updated'),
        ),
        kind: ToastKind.success,
      );
    } catch (error) {
      // Stable user-facing copy; the relay/crypto detail goes to the log.
      debugPrint('[RemindMeLater] createReminder failed: $error');
      messenger.showFeedbackSnackBar(
        const SnackBar(content: Text('Couldn’t set reminder. Try again.')),
        kind: ToastKind.error,
      );
    }
  }

  showBuzzModalBottomSheet<void>(
    context: context,
    title: 'Remind me about this message',
    showDragHandle: true,
    builder: (sheetContext) => SafeArea(
      child: IconTheme.merge(
        data: const IconThemeData(size: 22),
        child: SingleChildScrollView(
          child: Padding(
            padding: const EdgeInsets.fromLTRB(
              Grid.gutter,
              0,
              Grid.gutter,
              Grid.xs,
            ),
            child: ReminderTimeChoices(
              onChosen: (choice) {
                Navigator.of(sheetContext).pop();
                unawaited(submit(choice.getTimestamp()));
              },
            ),
          ),
        ),
      ),
    ),
  );
}
