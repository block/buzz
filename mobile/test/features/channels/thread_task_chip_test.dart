import 'package:buzz/features/channels/thread_detail_page/thread_task_chip.dart';
import 'package:buzz/shared/tasks/task.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

Task _task({
  String id = 'task-1',
  String title = 'A thread task',
  TaskStatus status = TaskStatus.todo,
  int updatedAt = 1786000000,
}) => Task(
  id: id,
  title: title,
  status: status,
  priority: 0,
  createdAt: DateTime.fromMillisecondsSinceEpoch(
    1786000000 * 1000,
    isUtc: true,
  ),
  updatedAt: DateTime.fromMillisecondsSinceEpoch(updatedAt * 1000, isUtc: true),
  channelId: 'channel-1',
  sourceRef: 'event-1',
);

Widget _app(Widget child) => MaterialApp(
  theme: AppTheme.light(),
  home: Scaffold(body: child),
);

final _chipTitle = find.byKey(const ValueKey('thread-task-chip-title'));
final _chipStatus = find.byKey(const ValueKey('thread-task-chip-status'));
final _chipMore = find.byKey(const ValueKey('thread-task-chip-more'));

void main() {
  group('clampThreadTaskTitle', () {
    test('leaves a short title untouched', () {
      expect(clampThreadTaskTitle('Ship it'), 'Ship it');
    });

    test('trims surrounding whitespace so the chip cannot be padded', () {
      expect(clampThreadTaskTitle('  Ship it \n'), 'Ship it');
    });

    test('clamps an over-long untrusted title', () {
      final clamped = clampThreadTaskTitle('a' * 500);
      expect(clamped.runes.length, threadTaskChipTitleChars);
    });

    // The relay counts characters via `chars().count()`, so the client guard
    // must count runes too. Clamping by String.length would slice a surrogate
    // pair in half and render a replacement character.
    test('counts runes, not UTF-16 code units', () {
      final clamped = clampThreadTaskTitle('😀' * 200);
      expect(clamped.runes.length, threadTaskChipTitleChars);
      expect(clamped.runes.every((rune) => rune == '😀'.runes.first), isTrue);
    });
  });

  group('ThreadTaskChip', () {
    testWidgets('shows the task title and status', (tester) async {
      await tester.pumpWidget(
        _app(
          ThreadTaskChip(
            task: _task(title: 'Add a keepalive ping', status: TaskStatus.done),
          ),
        ),
      );

      expect(tester.widget<Text>(_chipTitle).data, 'Add a keepalive ping');
      expect(tester.widget<Text>(_chipStatus).data, 'Done');
    });

    // The whole point of the re-scope: mobile has no task-viewing surface, so
    // this chip must never look tappable or dead-end a tap.
    testWidgets('is NON-INTERACTIVE: no tap handler in the widget tree', (
      tester,
    ) async {
      await tester.pumpWidget(_app(ThreadTaskChip(task: _task())));

      expect(find.byType(InkWell), findsNothing);
      expect(find.byType(GestureDetector), findsNothing);
      expect(find.byType(TextButton), findsNothing);
      expect(find.byType(IconButton), findsNothing);
      // Belt and braces: nothing in the subtree claims a tap gesture.
      expect(
        find.byWidgetPredicate(
          (widget) =>
              widget is RawGestureDetector &&
              widget.gestures.containsKey(TapGestureRecognizer),
        ),
        findsNothing,
      );
    });

    testWidgets('tapping the chip changes nothing and does not throw', (
      tester,
    ) async {
      await tester.pumpWidget(_app(ThreadTaskChip(task: _task())));
      await tester.tap(_chipTitle, warnIfMissed: false);
      await tester.pumpAndSettle();

      expect(tester.takeException(), isNull);
      expect(_chipTitle, findsOneWidget);
    });

    testWidgets('renders an inert +N suffix for extra tasks', (tester) async {
      await tester.pumpWidget(
        _app(ThreadTaskChip(task: _task(), additionalCount: 1)),
      );

      expect(tester.widget<Text>(_chipMore).data, '+1');
      // A count, not a control.
      expect(find.byType(InkWell), findsNothing);
    });

    testWidgets('renders no +N suffix for a single task', (tester) async {
      await tester.pumpWidget(_app(ThreadTaskChip(task: _task())));
      expect(_chipMore, findsNothing);
    });

    testWidgets('clamps a hostile title rather than growing the header', (
      tester,
    ) async {
      await tester.pumpWidget(
        _app(ThreadTaskChip(task: _task(title: 'x' * 400))),
      );

      final rendered = tester.widget<Text>(_chipTitle);
      expect(rendered.data!.runes.length, threadTaskChipTitleChars);
      expect(rendered.maxLines, 1);
      expect(rendered.overflow, TextOverflow.ellipsis);
    });
  });

  group('threadTaskStatusLabel', () {
    test('labels every lifecycle state', () {
      expect(threadTaskStatusLabel(TaskStatus.todo), 'To do');
      expect(threadTaskStatusLabel(TaskStatus.inProgress), 'In progress');
      expect(threadTaskStatusLabel(TaskStatus.blocked), 'Blocked');
      expect(threadTaskStatusLabel(TaskStatus.done), 'Done');
      expect(threadTaskStatusLabel(TaskStatus.cancelled), 'Cancelled');
    });
  });
}
