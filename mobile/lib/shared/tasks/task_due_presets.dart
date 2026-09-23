/// Quick due-date presets for the "New task" sheet.
///
/// Deliberately built on [nextDayAt9am] and [daysUntilNextMonday] from the
/// reminder presets rather than re-deriving the arithmetic: those two carry the
/// "always strictly in the future" guarantee and the repo's definition of
/// "next Monday", and a second copy would drift from it.
///
/// Task due dates are days, not minutes, so the labels are coarser than the
/// reminder ones ("Tomorrow at 9am", not "In 30 minutes").
library;

import 'package:flutter/foundation.dart';

import '../reminders/reminder_time_presets.dart';

/// The hour a "later today" task is due, in local time.
const _endOfWorkdayHour = 17;

/// A labelled due-date shortcut.
@immutable
class TaskDuePreset {
  /// Pairs a user-facing [label] with the instant it resolves to.
  const TaskDuePreset({required this.label, required this.dueAt});

  /// Text shown in the sheet.
  final String label;

  /// The resolved due instant, in local time.
  final DateTime dueAt;
}

DateTime _fromSeconds(int seconds) =>
    DateTime.fromMillisecondsSinceEpoch(seconds * 1000);

/// Builds the presets offered at the moment the sheet opens.
///
/// "Today at 5pm" is omitted rather than rolled forward once the hour has
/// passed — silently turning it into tomorrow would make the label lie.
List<TaskDuePreset> taskDuePresets({DateTime? now}) {
  final current = now ?? DateTime.now();
  final endOfToday = DateTime(
    current.year,
    current.month,
    current.day,
    _endOfWorkdayHour,
  );
  return [
    if (endOfToday.isAfter(current))
      TaskDuePreset(label: 'Today at 5pm', dueAt: endOfToday),
    TaskDuePreset(
      label: 'Tomorrow at 9am',
      dueAt: _fromSeconds(nextDayAt9am(1, now: current)),
    ),
    TaskDuePreset(
      label: 'Next Monday at 9am',
      dueAt: _fromSeconds(
        nextDayAt9am(daysUntilNextMonday(current), now: current),
      ),
    ),
    TaskDuePreset(
      label: 'In a week',
      dueAt: _fromSeconds(nextDayAt9am(7, now: current)),
    ),
  ];
}
