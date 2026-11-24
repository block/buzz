import 'package:buzz/shared/theme/buzz_icons.dart';
import 'package:flutter/material.dart';
import 'package:intl/intl.dart';

import '../theme/theme.dart';
import '../widgets/feedback_snackbar.dart';
import '../widgets/modal_presentation.dart';
import '../widgets/sheet_action_section.dart';
import 'reminder_time_presets.dart';

/// The time choices every reminder sheet offers: the desktop-parity presets
/// plus a native date and time picker.
///
/// [onChosen] receives the picked preset, or a fixed-time choice for a custom
/// pick. A cancelled picker, or a time that is not in the future, chooses
/// nothing, so the surrounding sheet stays open for another try.
class ReminderTimeChoices extends StatelessWidget {
  const ReminderTimeChoices({super.key, required this.onChosen});

  final ValueChanged<ReminderTimePreset> onChosen;

  @override
  Widget build(BuildContext context) {
    return Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        SheetActionSection(
          children: [
            for (final preset in reminderTimePresets)
              ListTile(
                leading: const Icon(BuzzIcons.clock),
                title: Text(preset.label),
                onTap: () => onChosen(preset),
              ),
          ],
        ),
        SheetActionSection(
          children: [
            ListTile(
              leading: const Icon(BuzzIcons.calendarClock),
              title: const Text('Pick a date & time'),
              onTap: () async {
                final timestamp = await _pickCustomDateTime(context);
                if (timestamp == null) return;
                onChosen(customReminderTime(timestamp));
              },
            ),
          ],
        ),
      ],
    );
  }
}

/// A fixed [timestamp] (Unix seconds) as a choice, labelled with its date.
ReminderTimePreset customReminderTime(int timestamp) {
  final date = DateTime.fromMillisecondsSinceEpoch(timestamp * 1000);
  return ReminderTimePreset(
    label: DateFormat.MMMd().add_jm().format(date),
    getTimestamp: () => timestamp,
  );
}

/// Opens [ReminderTimeChoices] in a sheet of its own and resolves to the
/// chosen time, or null when the sheet is dismissed.
Future<ReminderTimePreset?> showReminderTimeChoicesSheet(BuildContext context) {
  return showBuzzModalBottomSheet<ReminderTimePreset>(
    context: context,
    title: 'When',
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
              onChosen: (choice) => Navigator.of(sheetContext).pop(choice),
            ),
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
  final date = await showBuzzDialog<DateTime>(
    context: context,
    builder: (_) => DatePickerDialog(
      initialDate: now,
      firstDate: DateTime(now.year, now.month, now.day),
      lastDate: now.add(const Duration(days: 365 * 2)),
    ),
  );
  if (date == null || !context.mounted) return null;

  final time = await showBuzzDialog<TimeOfDay>(
    context: context,
    builder: (_) =>
        const TimePickerDialog(initialTime: TimeOfDay(hour: 9, minute: 0)),
  );
  if (time == null) return null;

  final timestamp = combineCustomDateTime(date, time.hour, time.minute);
  if (timestamp == null && context.mounted) {
    ScaffoldMessenger.of(context).showFeedbackSnackBar(
      const SnackBar(content: Text('Pick a time in the future')),
      kind: ToastKind.error,
    );
  }
  return timestamp;
}
