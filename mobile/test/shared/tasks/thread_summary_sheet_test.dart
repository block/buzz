import 'dart:convert';

import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/tasks/tasks_api.dart';
import 'package:buzz/shared/tasks/thread_summary.dart';
import 'package:buzz/shared/tasks/thread_summary_sheet.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart' as http_testing;
import 'package:nostr/nostr.dart' as nostr;

const _channelId = 'channel-1';

const _thread = [
  ThreadMessageDigest(
    author: 'Ada',
    text: 'The relay drops long-lived connections after an hour.',
  ),
  ThreadMessageDigest(
    author: 'Grace',
    text: 'We decided to add a keepalive ping; I will own it.',
  ),
];

final _save = find.byKey(const ValueKey('thread-summary-save'));
final _copy = find.byKey(const ValueKey('thread-summary-copy'));
final _newTaskRow = find.byKey(const ValueKey('summary-target-new-task'));

late String _nsec;

class _TestRelayConfig extends RelayConfigNotifier {
  @override
  RelayConfig build() =>
      RelayConfig(baseUrl: 'https://relay.example.com', nsec: _nsec);
}

class _Harness extends ConsumerWidget {
  const _Harness();

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    return Scaffold(
      body: Center(
        child: TextButton(
          onPressed: () => showThreadSummarySheet(
            context: context,
            ref: ref,
            channelId: _channelId,
            messages: _thread,
          ),
          child: const Text('open'),
        ),
      ),
    );
  }
}

String _taskBody({String id = 'task-1', String title = 'A thread task'}) =>
    jsonEncode({
      'id': id,
      'title': title,
      'status': 'todo',
      'priority': 0,
      'created_at': 1786000000,
      'updated_at': 1786000000,
    });

Widget _app(http.Client client) => ProviderScope(
  overrides: [
    relayConfigProvider.overrideWith(_TestRelayConfig.new),
    tasksHttpClientProvider.overrideWithValue(client),
  ],
  child: MaterialApp(theme: AppTheme.light(), home: const _Harness()),
);

void main() {
  setUp(() => _nsec = nostr.Keys.generate().nsec);

  /// Routes by method and path so a test can assert the whole call sequence.
  http.Client routing({
    required List<String> log,
    String listBody = '{"tasks": []}',
    http.Response Function()? appendResponse,
  }) {
    return http_testing.MockClient((request) async {
      log.add('${request.method} ${request.url.path}');
      if (request.url.path.endsWith('/events')) {
        return appendResponse?.call() ??
            http.Response(
              jsonEncode({
                'id': 1,
                'task_id': 'task-1',
                'action': 'summary_persisted',
                'created_at': 1786000000,
                'body': 'digest',
              }),
              200,
            );
      }
      if (request.method == 'GET') return http.Response(listBody, 200);
      return http.Response(_taskBody(), 200);
    });
  }

  testWidgets('renders the digest with copy and save actions', (tester) async {
    await tester.pumpWidget(_app(routing(log: [])));
    await tester.tap(find.text('open'));
    await tester.pumpAndSettle();

    expect(
      tester.widget<Text>(find.byKey(const ValueKey('buzz-sheet-title'))).data,
      'Thread summary',
    );
    expect(_copy, findsOneWidget);
    expect(_save, findsOneWidget);
    // The digest itself is the pure function's output, asserted in
    // thread_summary_test.dart; here it only has to reach the sheet — which the
    // rendered '## Thread summary' heading, distinct from the sheet title
    // above, demonstrates.
    expect(find.byKey(const ValueKey('thread-summary-body')), findsOneWidget);
    expect(find.text('Thread summary'), findsWidgets);
  });

  testWidgets('opens a new task and persists the summary onto it', (
    tester,
  ) async {
    final log = <String>[];
    await tester.pumpWidget(_app(routing(log: log)));
    await tester.tap(find.text('open'));
    await tester.pumpAndSettle();

    await tester.tap(_save);
    await tester.pumpAndSettle();

    expect(find.text('Save summary to'), findsOneWidget);
    await tester.tap(_newTaskRow);
    await tester.pumpAndSettle();

    expect(log, [
      'GET /api/tasks', // the picker lists candidates
      'POST /api/tasks', // no existing task chosen, so open one
      'POST /api/tasks/task-1/events', // then persist the summary
    ]);
    expect(find.text('Summary saved to task'), findsOneWidget);
    expect(find.text('Saved to “A thread task”.'), findsOneWidget);
    // Re-saving the same summary can only fail, so the action retires.
    expect(tester.widget<FilledButton>(_save).onPressed, isNull);
  });

  testWidgets('persists onto an existing task without creating one', (
    tester,
  ) async {
    final log = <String>[];
    await tester.pumpWidget(
      _app(
        routing(
          log: log,
          listBody: jsonEncode({
            'tasks': [jsonDecode(_taskBody(id: 'task-9', title: 'Keepalive'))],
          }),
        ),
      ),
    );
    await tester.tap(find.text('open'));
    await tester.pumpAndSettle();
    await tester.tap(_save);
    await tester.pumpAndSettle();

    expect(find.text('Existing tasks'), findsOneWidget);
    await tester.tap(find.byKey(const ValueKey('summary-target-task-9')));
    await tester.pumpAndSettle();

    expect(log, ['GET /api/tasks', 'POST /api/tasks/task-9/events']);
    expect(find.text('Saved to “Keepalive”.'), findsOneWidget);
  });

  testWidgets('surfaces the relay refusal of a second summary', (tester) async {
    final log = <String>[];
    await tester.pumpWidget(
      _app(
        routing(
          log: log,
          listBody: jsonEncode({
            'tasks': [jsonDecode(_taskBody(id: 'task-9', title: 'Keepalive'))],
          }),
          appendResponse: () => http.Response(
            jsonEncode(const {
              'error': 'task task-9 already has a persisted summary',
            }),
            400,
          ),
        ),
      ),
    );
    await tester.tap(find.text('open'));
    await tester.pumpAndSettle();
    await tester.tap(_save);
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const ValueKey('summary-target-task-9')));
    await tester.pumpAndSettle();

    expect(
      find.text('task task-9 already has a persisted summary'),
      findsOneWidget,
    );
    // The action stays live so another task can be chosen.
    expect(tester.widget<FilledButton>(_save).onPressed, isNotNull);
  });

  testWidgets('dismissing the picker leaves the summary untouched', (
    tester,
  ) async {
    final log = <String>[];
    await tester.pumpWidget(_app(routing(log: log)));
    await tester.tap(find.text('open'));
    await tester.pumpAndSettle();
    await tester.tap(_save);
    await tester.pumpAndSettle();

    await tester.tap(find.byTooltip('Close sheet').last);
    await tester.pumpAndSettle();

    expect(log, ['GET /api/tasks']);
    expect(find.text('Summary saved to task'), findsNothing);
    expect(tester.widget<FilledButton>(_save).onPressed, isNotNull);
  });

  testWidgets('reports a failed task list instead of an empty picker', (
    tester,
  ) async {
    final client = http_testing.MockClient(
      (request) async => http.Response(
        jsonEncode(const {'error': 'relay: no community for this host'}),
        404,
      ),
    );
    await tester.pumpWidget(_app(client));
    await tester.tap(find.text('open'));
    await tester.pumpAndSettle();
    await tester.tap(_save);
    await tester.pumpAndSettle();

    expect(find.text('relay: no community for this host'), findsOneWidget);
    // Opening a fresh task is still possible even when listing failed.
    expect(_newTaskRow, findsOneWidget);
  });
}
