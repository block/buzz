import 'package:buzz/shared/tasks/task_due_presets.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  group('taskDuePresets', () {
    test('offers "Today at 5pm" while the hour is still ahead', () {
      final presets = taskDuePresets(now: DateTime(2026, 8, 20, 9));
      expect(presets.first.label, 'Today at 5pm');
      expect(presets.first.dueAt, DateTime(2026, 8, 20, 17));
    });

    test('drops "Today at 5pm" once it has passed, rather than lying', () {
      final presets = taskDuePresets(now: DateTime(2026, 8, 20, 18));
      expect(
        presets.map((preset) => preset.label),
        isNot(contains('Today at 5pm')),
      );
      expect(presets.first.label, 'Tomorrow at 9am');
    });

    test('every preset is strictly in the future', () {
      for (final now in [
        DateTime(2026, 8, 17, 8, 59), // Monday morning
        DateTime(2026, 8, 17, 9, 1), // Monday just after 9am
        DateTime(2026, 8, 22, 23, 59), // Saturday night
      ]) {
        for (final preset in taskDuePresets(now: now)) {
          expect(
            preset.dueAt.isAfter(now),
            isTrue,
            reason: '${preset.label} from $now resolved to ${preset.dueAt}',
          );
        }
      }
    });

    test('"Next Monday" means the following week when today is Monday', () {
      final now = DateTime(2026, 8, 17, 10); // a Monday
      expect(now.weekday, DateTime.monday);
      final monday = taskDuePresets(
        now: now,
      ).firstWhere((preset) => preset.label == 'Next Monday at 9am');
      expect(monday.dueAt, DateTime(2026, 8, 24, 9));
    });

    test('"In a week" lands seven days out at 9am', () {
      final presets = taskDuePresets(now: DateTime(2026, 8, 20, 9));
      final week = presets.firstWhere((preset) => preset.label == 'In a week');
      expect(week.dueAt, DateTime(2026, 8, 27, 9));
    });

    test('labels are unique, so chip selection is unambiguous', () {
      final labels = taskDuePresets(
        now: DateTime(2026, 8, 20, 9),
      ).map((preset) => preset.label).toList();
      expect(labels.toSet(), hasLength(labels.length));
    });
  });
}
