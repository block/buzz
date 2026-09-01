/// Tests for the HW-007 task status picker.
///
/// The picker is deliberately network-free, so these tests pump the widget
/// directly with no relay, no `TasksApi` and no sheet route. What they pin down
/// is the SELECTION CONTRACT the detail sheet depends on: every status is
/// offered, the current one is marked, and the callback fires with the tapped
/// value INCLUDING when it equals the current status — because declining the
/// no-op is the caller's job, not the picker's.
library;

import 'package:buzz/features/channels/thread_detail_page/task_status_picker.dart';
import 'package:buzz/features/channels/thread_detail_page/thread_task_chip.dart';
import 'package:buzz/shared/tasks/task.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

Future<List<TaskStatus>> _pumpPicker(
  WidgetTester tester, {
  required TaskStatus current,
}) async {
  final selected = <TaskStatus>[];
  await tester.pumpWidget(
    ProviderScope(
      child: MaterialApp(
        theme: AppTheme.light(),
        home: Scaffold(
          body: TaskStatusPicker(current: current, onSelected: selected.add),
        ),
      ),
    ),
  );
  await tester.pumpAndSettle();
  return selected;
}

Finder _option(TaskStatus status) =>
    find.byKey(ValueKey('task-status-option-${status.wireValue}'));

Finder _marker(TaskStatus status) =>
    find.byKey(ValueKey('task-status-current-${status.wireValue}'));

void main() {
  group('TaskStatusPicker', () {
    testWidgets('offers every status, with a row per TaskStatus value', (
      tester,
    ) async {
      await _pumpPicker(tester, current: TaskStatus.todo);

      // Enumerated from the enum itself, so a status added later fails this
      // test rather than silently becoming unreachable on mobile.
      for (final status in TaskStatus.values) {
        expect(_option(status), findsOneWidget);
        expect(find.text(threadTaskStatusLabel(status)), findsOneWidget);
      }
      expect(TaskStatus.values.length, 5);
    });

    testWidgets('marks exactly the current status, and only that one', (
      tester,
    ) async {
      await _pumpPicker(tester, current: TaskStatus.blocked);

      expect(_marker(TaskStatus.blocked), findsOneWidget);
      for (final status in TaskStatus.values) {
        if (status == TaskStatus.blocked) continue;
        expect(_marker(status), findsNothing);
      }
    });

    testWidgets('moves the marker when the current status differs', (
      tester,
    ) async {
      await _pumpPicker(tester, current: TaskStatus.done);

      expect(_marker(TaskStatus.done), findsOneWidget);
      expect(_marker(TaskStatus.todo), findsNothing);
    });

    testWidgets('reports the tapped status to the caller', (tester) async {
      final selected = await _pumpPicker(tester, current: TaskStatus.todo);

      await tester.tap(_option(TaskStatus.inProgress));
      await tester.pumpAndSettle();

      expect(selected, [TaskStatus.inProgress]);
    });

    // The picker does NOT swallow the no-op. Suppressing it here would hide
    // the decision from the sheet, which is the component that owns whether a
    // request is sent; the sheet declines it instead.
    testWidgets('still reports a tap on the CURRENT status', (tester) async {
      final selected = await _pumpPicker(tester, current: TaskStatus.todo);

      await tester.tap(_option(TaskStatus.todo));
      await tester.pumpAndSettle();

      expect(selected, [TaskStatus.todo]);
    });

    // No per-user gate: the relay authorizes PATCH on channel membership with
    // no ownership check, so hiding or disabling an option would misrepresent
    // the real invariant.
    testWidgets('disables nothing — every option is tappable', (tester) async {
      final selected = await _pumpPicker(tester, current: TaskStatus.todo);

      for (final status in TaskStatus.values) {
        await tester.tap(_option(status));
        await tester.pumpAndSettle();
      }

      expect(selected, TaskStatus.values);
    });

    testWidgets('marks the current status by semantics, not weight alone', (
      tester,
    ) async {
      await _pumpPicker(tester, current: TaskStatus.cancelled);

      final icon = tester.widget<Icon>(_marker(TaskStatus.cancelled));
      expect(icon.semanticLabel, 'Current status');
    });
  });
}
