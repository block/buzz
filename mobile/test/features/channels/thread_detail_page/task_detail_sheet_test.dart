/// Tests for the HW-005 read-only task detail sheet.
///
/// The tests pump the sheet through its real entry point (`showTaskDetailSheet`)
/// against a REAL `TasksApi` backed by a `MockClient`, so the JSON the sheet
/// parses is exactly the JSON `getTask` produces — no stubbed provider
/// returning pre-built models.
library;

import 'dart:async';
import 'dart:convert';

import 'package:buzz/features/channels/thread_detail_page/task_detail_sheet.dart';
import 'package:buzz/features/channels/thread_detail_page/thread_task_chip.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/tasks/tasks_api.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart' as http_testing;
import 'package:nostr/nostr.dart' as nostr;

const _baseUrl = 'https://relay.example.com';

Map<String, dynamic> _taskJson({
  String id = 'task-1',
  String title = 'Ship the digest contract',
  String status = 'in_progress',
}) => {
  'id': id,
  'title': title,
  'status': status,
  'priority': 0,
  'created_at': 1786000000,
  'updated_at': 1786000060,
};

Map<String, dynamic> _eventJson({
  required int id,
  String action = 'commented',
  String? actor,
  String? fromStatus,
  String? toStatus,
  String? body,
}) => {
  'id': id,
  'task_id': 'task-1',
  'action': action,
  'created_at': 1786000000 + id,
  'actor': ?actor,
  'from_status': ?fromStatus,
  'to_status': ?toStatus,
  'body': ?body,
};

http.Response _detailResponse({
  String title = 'Ship the digest contract',
  List<Map<String, dynamic>> events = const [],
}) => http.Response(
  jsonEncode({'task': _taskJson(title: title), 'events': events}),
  200,
);

/// The relay's real 404 body for both "missing" and "channel not accessible"
/// (tasks.rs `enforce_channel_access` deliberately 404s invisible channels).
const _relay404Body = '{"error":"task not found"}';

class _StaticRelayConfig extends RelayConfigNotifier {
  _StaticRelayConfig(this._nsec);

  final String? _nsec;

  @override
  RelayConfig build() => RelayConfig(baseUrl: _baseUrl, nsec: _nsec);
}

/// A screen with one button that opens the sheet through the same entry point
/// the page uses, so the tap-to-open path and the sheet's own lifecycle are
/// both exercised for real.
class _TaskDetailSheetOpener extends HookConsumerWidget {
  const _TaskDetailSheetOpener();

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    return Scaffold(
      body: Center(
        child: TextButton(
          key: const ValueKey('open-task-detail'),
          onPressed: () => showTaskDetailSheet(
            context: context,
            ref: ref,
            taskId: 'task-1',
          ),
          child: const Text('Open task detail'),
        ),
      ),
    );
  }
}

Future<void> _pumpSheet(
  WidgetTester tester, {
  required Future<http.Response> Function(http.Request request) handler,
  required String nsec,
  bool dark = false,
}) async {
  await tester.pumpWidget(
    ProviderScope(
      overrides: [
        tasksHttpClientProvider.overrideWith(
          (ref) => http_testing.MockClient(handler),
        ),
        // The static config avoids the real RelayConfigNotifier's dependency
        // on the active-community provider chain.
        relayConfigProvider.overrideWith(() => _StaticRelayConfig(nsec)),
      ],
      child: MaterialApp(
        theme: dark ? AppTheme.dark() : AppTheme.light(),
        home: const _TaskDetailSheetOpener(),
      ),
    ),
  );
  await tester.pump();
}

final _titleFinder = find.byKey(const ValueKey('task-detail-title'));
final _statusFinder = find.byKey(const ValueKey('task-detail-status'));
final _summaryFinder = find.byKey(const ValueKey('task-detail-summary'));
final _errorFinder = find.byKey(const ValueKey('task-detail-error'));
final _retryFinder = find.byKey(const ValueKey('task-detail-retry'));
final _loadingFinder = find.byKey(const ValueKey('task-detail-loading'));
final _openButton = find.byKey(const ValueKey('open-task-detail'));

Future<void> _openSheet(WidgetTester tester) async {
  await tester.tap(_openButton);
  await tester.pump(); // sheet route entry animation frame
}

void main() {
  late String nsec;

  setUp(() => nsec = nostr.Keys.generate().nsec);

  group('clampTaskDetailBody', () {
    test('leaves a short body untouched', () {
      expect(clampTaskDetailBody('Looks good'), 'Looks good');
    });

    test('trims surrounding whitespace', () {
      expect(clampTaskDetailBody('  Looks good \n'), 'Looks good');
    });

    // Runes, not UTF-16 code units: clamping by String.length would split an
    // astral-plane character in half and render a replacement character.
    test('counts runes, not code units', () {
      final clamped = clampTaskDetailBody('😀' * 600);
      expect(clamped.runes.length, taskDetailBodyChars);
      expect(clamped.runes.every((rune) => rune == '😀'.runes.first), isTrue);
    });
  });

  group('taskWireStatusLabel', () {
    test('labels every known wire status', () {
      expect(taskWireStatusLabel('todo'), 'To do');
      expect(taskWireStatusLabel('in_progress'), 'In progress');
      expect(taskWireStatusLabel('blocked'), 'Blocked');
      expect(taskWireStatusLabel('done'), 'Done');
      expect(taskWireStatusLabel('cancelled'), 'Cancelled');
    });

    // A transition this build has never heard of must pass through verbatim
    // rather than being guessed into a known label: history must never be
    // fabricated by a client that predates a new status.
    test('passes an unknown status through verbatim', () {
      expect(taskWireStatusLabel('escalated'), 'escalated');
    });

    test('returns null for a missing status', () {
      expect(taskWireStatusLabel(null), isNull);
    });
  });

  group('TaskDetailSheet', () {
    testWidgets('shows title, status, summary and one row per event', (
      tester,
    ) async {
      await _pumpSheet(
        tester,
        nsec: nsec,
        handler: (request) async => _detailResponse(
          events: [
            _eventJson(id: 1, action: 'created', actor: 'a' * 64),
            _eventJson(
              id: 2,
              action: 'status_changed',
              actor: 'a' * 64,
              fromStatus: 'todo',
              toStatus: 'in_progress',
            ),
            _eventJson(
              id: 3,
              action: 'summary_persisted',
              body: 'Agent digest of the thread.',
            ),
            _eventJson(
              id: 4,
              action: 'commented',
              actor: 'b' * 64,
              body: 'FYI',
            ),
          ],
        ),
      );
      await _openSheet(tester);
      await tester.pumpAndSettle();

      expect(
        tester.widget<Text>(_titleFinder).data,
        'Ship the digest contract',
      );
      expect(tester.widget<Text>(_statusFinder).data, 'In progress');
      expect(_summaryFinder, findsOneWidget);
      // The summary is promoted to its own card, so it must NOT also render
      // as a history row — that would show the same text twice.
      expect(find.text('Agent digest of the thread.'), findsOneWidget);
      expect(find.byKey(const ValueKey('task-event-row-1')), findsOneWidget);
      expect(find.byKey(const ValueKey('task-event-row-2')), findsOneWidget);
      expect(find.byKey(const ValueKey('task-event-row-3')), findsNothing);
      expect(find.byKey(const ValueKey('task-event-row-4')), findsOneWidget);
      // A status-change row states the transition.
      expect(find.textContaining('To do'), findsOneWidget);
      // 'In progress' appears in the header status and the transition row.
      expect(find.textContaining('In progress'), findsWidgets);
      // Actor pubkeys render truncated, plain text.
      final actorText = tester.widget<Text>(
        find.byKey(const ValueKey('task-event-actor-1')),
      );
      expect(actorText.data, '${'a' * 8}\u2026');
    });

    testWidgets('omits the summary section entirely when absent', (
      tester,
    ) async {
      await _pumpSheet(
        tester,
        nsec: nsec,
        handler: (request) async =>
            _detailResponse(events: [_eventJson(id: 1, action: 'created')]),
      );
      await _openSheet(tester);
      await tester.pumpAndSettle();

      expect(_titleFinder, findsOneWidget);
      expect(_summaryFinder, findsNothing);
      // No placeholder, no "no summary" claim, no layout hole.
      expect(find.textContaining('Summary'), findsNothing);
    });

    testWidgets('renders an unknown action as a neutral row, not an error', (
      tester,
    ) async {
      await _pumpSheet(
        tester,
        nsec: nsec,
        handler: (request) async => _detailResponse(
          events: [
            _eventJson(id: 7, action: 'escalated_by_policy'),
            _eventJson(
              id: 8,
              action: 'status_changed',
              fromStatus: 'todo',
              toStatus: 'escalated',
            ),
          ],
        ),
      );
      await _openSheet(tester);
      await tester.pumpAndSettle();

      // The unknown ACTION renders verbatim as its own row...
      expect(find.text('escalated_by_policy'), findsOneWidget);
      // ...and the unknown TO-status inside a transition renders verbatim
      // too, never guessed into a known label. The row key sits on the row's
      // Padding wrapper, so read the Text descendant.
      expect(
        tester
            .widget<Text>(
              find
                  .descendant(
                    of: find.byKey(const ValueKey('task-event-row-8')),
                    matching: find.byType(Text),
                  )
                  .first,
            )
            .data,
        'To do → escalated',
      );
      expect(_errorFinder, findsNothing);
      expect(tester.takeException(), isNull);
    });

    testWidgets('shows an error row with retry on failure, then recovers', (
      tester,
    ) async {
      var calls = 0;
      await _pumpSheet(
        tester,
        nsec: nsec,
        handler: (request) async {
          calls++;
          if (calls == 1) {
            return http.Response(_relay404Body, 404);
          }
          return _detailResponse(
            events: [_eventJson(id: 1, action: 'created')],
          );
        },
      );
      await _openSheet(tester);
      await tester.pumpAndSettle();

      expect(_errorFinder, findsOneWidget);
      // No false "no task" claim, and no task content either.
      expect(_titleFinder, findsNothing);
      expect(
        find.byKey(const ValueKey('task-detail-error-message')),
        findsOneWidget,
      );

      await tester.tap(_retryFinder);
      await tester.pumpAndSettle();

      expect(calls, 2);
      expect(_titleFinder, findsOneWidget);
      expect(_errorFinder, findsNothing);
    });

    testWidgets('a 404 renders the same error row as any other failure', (
      tester,
    ) async {
      // Both must render the same error row: the sheet must not become an
      // existence oracle by revealing WHICH failure happened.
      await _pumpSheet(
        tester,
        nsec: nsec,
        handler: (request) async => http.Response(_relay404Body, 404),
      );
      await _openSheet(tester);
      await tester.pumpAndSettle();

      expect(_errorFinder, findsOneWidget);
      expect(_titleFinder, findsNothing);
      expect(_retryFinder, findsOneWidget);
    });

    testWidgets('a 500 renders the same error row as a 404', (tester) async {
      await _pumpSheet(
        tester,
        nsec: nsec,
        handler: (request) async => http.Response('{"error":"internal"}', 500),
      );
      await _openSheet(tester);
      await tester.pumpAndSettle();

      expect(_errorFinder, findsOneWidget);
      expect(_titleFinder, findsNothing);
      expect(_retryFinder, findsOneWidget);
    });

    testWidgets('shows a spinner while the fetch is in flight', (tester) async {
      final response = Completer<http.Response>();
      await _pumpSheet(
        tester,
        nsec: nsec,
        handler: (request) => response.future,
      );
      await _openSheet(tester);
      await tester.pump();

      expect(_loadingFinder, findsOneWidget);

      // Complete the in-flight request and settle so no timers or futures
      // dangle past the test boundary (the spinner animates on a timer).
      response.complete(_detailResponse());
      await tester.pumpAndSettle();
      expect(_titleFinder, findsOneWidget);
    });

    testWidgets('clamps a hostile 10k-char title and body, no exceptions', (
      tester,
    ) async {
      await _pumpSheet(
        tester,
        nsec: nsec,
        handler: (request) async => _detailResponse(
          title: 'x' * 10000,
          events: [
            _eventJson(id: 1, action: 'commented', body: 'y' * 10000),
          ],
        ),
      );
      await _openSheet(tester);
      await tester.pumpAndSettle();

      expect(
        tester.widget<Text>(_titleFinder).data!.runes.length,
        threadTaskChipTitleChars,
      );
      // The comment row renders its clamped body without overflowing.
      expect(find.byKey(const ValueKey('task-event-row-1')), findsOneWidget);
      expect(tester.takeException(), isNull);
    });

    testWidgets('markdown and link attempts render as plain text', (
      tester,
    ) async {
      await _pumpSheet(
        tester,
        nsec: nsec,
        handler: (request) async => _detailResponse(
          events: [
            _eventJson(
              id: 1,
              action: 'commented',
              body: '[click me](https://evil.example) <b>bold</b>',
            ),
          ],
        ),
      );
      await _openSheet(tester);
      await tester.pumpAndSettle();

      // The raw string renders; nothing parses or follows it.
      expect(
        find.textContaining('[click me](https://evil.example)'),
        findsOneWidget,
      );
      expect(find.byType(RichText), findsWidgets);
      // No URL link recognizer anywhere in the sheet.
      expect(
        find.byWidgetPredicate(
          (widget) =>
              widget is RichText &&
              widget.text.toPlainText().contains('click me'),
        ),
        findsOneWidget,
      );
    });

    testWidgets('dismissing the sheet returns to the host page', (
      tester,
    ) async {
      await _pumpSheet(
        tester,
        nsec: nsec,
        handler: (request) async => _detailResponse(),
      );
      await _openSheet(tester);
      await tester.pumpAndSettle();

      expect(_titleFinder, findsOneWidget);

      // Close via the shared sheet header's close button.
      final closeButton = find.byWidgetPredicate(
        (widget) => widget is IconButton && widget.tooltip == 'Close sheet',
      );
      expect(closeButton, findsOneWidget);
      await tester.tap(closeButton);
      await tester.pumpAndSettle();

      expect(_titleFinder, findsNothing);
      expect(_openButton, findsOneWidget);
    });
  });
}
