import 'package:flutter/material.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../theme/theme.dart';
import '../widgets/sheet_divider.dart';
import 'reminder_service.dart';
import 'reminder_time_presets.dart';

/// Bottom sheet for "Remind me later": desktop-parity presets plus a native
/// date/time picker, publishing the same author-private kind-30300 reminder
/// events desktop reads.
void showRemindMeLaterSheet({
  required BuildContext context,
  required WidgetRef ref,
  required ReminderTarget target,
}) {
  final service = ref.read(reminderServiceProvider);
  final messenger = ScaffoldMessenger.of(context);
  if (service == null) {
    messenger.showSnackBar(
      const SnackBar(content: Text('Sign in to set reminders')),
    );
    return;
  }

  Future<void> submit(int notBefore) async {
    try {
      await service.createReminder(target: target, notBefore: notBefore);
      messenger.showSnackBar(const SnackBar(content: Text('Reminder set')));
    } catch (error) {
      // Stable user-facing copy; the relay/crypto detail goes to the log.
      debugPrint('[RemindMeLater] createReminder failed: $error');
      messenger.showSnackBar(
        const SnackBar(content: Text('Couldn’t set reminder. Try again.')),
      );
    }
  }

  showModalBottomSheet<void>(
    context: context,
    showDragHandle: true,
    builder: (sheetContext) => SafeArea(
      child: SingleChildScrollView(
        child: Padding(
          padding: const EdgeInsets.fromLTRB(
            Grid.gutter,
            0,
            Grid.gutter,
            Grid.xs,
          ),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Padding(
                padding: const EdgeInsets.only(
                  left: Grid.half,
                  bottom: Grid.xxs,
                ),
                child: Text(
                  'Remind me about this message',
                  style: Theme.of(sheetContext).textTheme.titleSmall,
                ),
              ),
              for (final preset in reminderTimePresets)
                ListTile(
                  leading: const Icon(LucideIcons.clock),
                  title: Text(preset.label),
                  onTap: () {
                    Navigator.of(sheetContext).pop();
                    submit(preset.getTimestamp());
                  },
                ),
              const SheetDivider(),
              ListTile(
                leading: const Icon(LucideIcons.calendarClock),
                title: const Text('Pick a date & time'),
                onTap: () async {
                  final navigator = Navigator.of(sheetContext);
                  final timestamp = await _pickCustomDateTime(context);
                  // Cancelled or not-in-the-future: keep the preset sheet
                  // open so retrying doesn't mean long-pressing again.
                  if (timestamp == null) return;
                  if (navigator.mounted) navigator.pop();
                  await submit(timestamp);
                },
              ),
            ],
          ),
        ),
      ),
    ),
  );
}

/// Native date + time pickers chained together. Returns a future Unix
/// timestamp in seconds, or null when cancelled or not in the future.
Future<int?> _pickCustomDateTime(BuildContext context) async {
  final now = DateTime.now();
  final date = await showDatePicker(
    context: context,
    initialDate: now,
    firstDate: DateTime(now.year, now.month, now.day),
    lastDate: now.add(const Duration(days: 365 * 2)),
  );
  if (date == null || !context.mounted) return null;

  final time = await showTimePicker(
    context: context,
    initialTime: const TimeOfDay(hour: 9, minute: 0),
  );
  if (time == null) return null;

  final timestamp = combineCustomDateTime(date, time.hour, time.minute);
  if (timestamp == null && context.mounted) {
    ScaffoldMessenger.of(
      context,
    ).showSnackBar(const SnackBar(content: Text('Pick a time in the future')));
  }
  return timestamp;
}
