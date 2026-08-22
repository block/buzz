import 'dart:convert';

import 'package:buzz/shared/tasks/task.dart';
import 'package:buzz/shared/tasks/tasks_api.dart';
import 'package:crypto/crypto.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart' as http_testing;
import 'package:nostr/nostr.dart' as nostr;

const _baseUrl = 'https://relay.example.com';

Map<String, dynamic> _taskJson({String id = 'task-1'}) => {
  'id': id,
  'title': 'Ship it',
  'status': 'todo',
  'priority': 0,
  'created_at': 1786000000,
  'updated_at': 1786000000,
};

/// Decodes the `Authorization: Nostr <base64>` header back into its event.
Map<String, dynamic> _nip98Event(http.Request request) {
  final header = request.headers['authorization'];
  expect(header, startsWith('Nostr '));
  final json = utf8.decode(base64.decode(header!.substring('Nostr '.length)));
  return jsonDecode(json) as Map<String, dynamic>;
}

String? _tag(Map<String, dynamic> event, String name) {
  for (final tag in event['tags'] as List) {
    final entry = (tag as List).cast<String>();
    if (entry.first == name) return entry[1];
  }
  return null;
}

void main() {
  late String nsec;

  setUp(() => nsec = nostr.Keys.generate().nsec);

  TasksApi apiWith(
    Future<http.Response> Function(http.Request request) handler, {
    String? signingKey,
  }) => TasksApi(
    httpClient: http_testing.MockClient(handler),
    baseUrl: _baseUrl,
    nsec: signingKey ?? nsec,
  );

  group('createTask', () {
    test('posts the payload with a NIP-98 signature over that body', () async {
      late http.Request captured;
      final api = apiWith((request) async {
        captured = request;
        return http.Response(jsonEncode(_taskJson()), 200);
      });

      final task = await api.createTask(
        title: 'Ship it',
        channelId: 'channel-1',
        sourceRef: 'event-1',
      );

      expect(captured.method, 'POST');
      expect(captured.url, Uri.parse('$_baseUrl/api/tasks'));
      expect(captured.headers['content-type'], 'application/json');
      expect(jsonDecode(captured.body), {
        'title': 'Ship it',
        'channel_id': 'channel-1',
        'source_ref': 'event-1',
        'source': 'mobile',
      });

      final event = _nip98Event(captured);
      expect(event['kind'], 27235);
      expect(_tag(event, 'u'), '$_baseUrl/api/tasks');
      expect(_tag(event, 'method'), 'POST');
      // The relay requires a payload tag on every write and rejects a
      // signature whose hash does not cover the body it arrived with.
      expect(
        _tag(event, 'payload'),
        sha256.convert(utf8.encode(captured.body)).toString(),
      );

      expect(task.id, 'task-1');
    });

    test('rejects an over-long title before sending anything', () async {
      var called = false;
      final api = apiWith((request) async {
        called = true;
        return http.Response('{}', 200);
      });

      await expectLater(
        api.createTask(title: 'a' * (maxTaskTitleChars + 1)),
        throwsA(isA<ArgumentError>()),
      );
      expect(called, isFalse);
    });
  });

  group('listTasks', () {
    test('signs the URL including its query string', () async {
      late http.Request captured;
      final api = apiWith((request) async {
        captured = request;
        return http.Response(
          jsonEncode({
            'tasks': [_taskJson()],
          }),
          200,
        );
      });

      final tasks = await api.listTasks(
        status: TaskStatus.inProgress,
        channelId: 'channel-1',
        limit: 20,
      );

      expect(captured.method, 'GET');
      expect(captured.url.path, '/api/tasks');
      expect(captured.url.queryParameters, {
        'status': 'in_progress',
        'channel': 'channel-1',
        'limit': '20',
      });
      // `request_path` rebuilds the expected URL from the path plus the raw
      // query, so a signature over the bare path would fail verification.
      expect(_tag(_nip98Event(captured), 'u'), captured.url.toString());
      expect(tasks.single.id, 'task-1');
    });

    test('omits filters that were not set', () async {
      late http.Request captured;
      final api = apiWith((request) async {
        captured = request;
        return http.Response(jsonEncode(const {'tasks': []}), 200);
      });

      expect(await api.listTasks(), isEmpty);
      expect(captured.url, Uri.parse('$_baseUrl/api/tasks'));
      expect(captured.url.query, isEmpty);
    });

    test('sends source_ref verbatim and signs the resulting URL', () async {
      late http.Request captured;
      final api = apiWith((request) async {
        captured = request;
        return http.Response(
          jsonEncode({
            'tasks': [_taskJson()],
          }),
          200,
        );
      });

      final tasks = await api.listTasks(
        channelId: 'channel-1',
        sourceRef: 'thread-head-aaa',
      );

      expect(captured.url.queryParameters, {
        'channel': 'channel-1',
        'source_ref': 'thread-head-aaa',
      });
      // The relay rebuilds the signed URL from path + raw query, so the
      // signature must cover source_ref exactly as it went out.
      expect(_tag(_nip98Event(captured), 'u'), captured.url.toString());
      expect(tasks.single.id, 'task-1');
    });

    test('emits no source_ref parameter when it is absent', () async {
      late http.Request captured;
      final api = apiWith((request) async {
        captured = request;
        return http.Response(jsonEncode(const {'tasks': []}), 200);
      });

      // A stray empty parameter would change the signed query for every
      // existing caller, so absence must stay absence.
      await api.listTasks(channelId: 'channel-1');
      expect(captured.url.queryParameters.containsKey('source_ref'), isFalse);
      expect(captured.url.query, 'channel=channel-1');
    });

    test('rejects a response whose task list is not a list', () async {
      final api = apiWith(
        (request) async => http.Response(jsonEncode(const {'tasks': 3}), 200),
      );
      await expectLater(api.listTasks(), throwsA(isA<FormatException>()));
    });
  });

  group('getTask', () {
    test('parses the task and its history', () async {
      final api = apiWith(
        (request) async => http.Response(
          jsonEncode({
            'task': _taskJson(),
            'events': [
              {
                'id': 1,
                'task_id': 'task-1',
                'action': 'created',
                'created_at': 1786000000,
              },
              {
                'id': 2,
                'task_id': 'task-1',
                'action': 'summary_persisted',
                'created_at': 1786000005,
                'body': '## Thread summary',
              },
            ],
          }),
          200,
        ),
      );

      final detail = await api.getTask('task-1');
      expect(detail.events, hasLength(2));
      expect(detail.summary?.body, '## Thread summary');
    });
  });

  group('appendTaskEvent', () {
    test('posts the action and body', () async {
      late http.Request captured;
      final api = apiWith((request) async {
        captured = request;
        return http.Response(
          jsonEncode({
            'id': 7,
            'task_id': 'task-1',
            'action': 'summary_persisted',
            'created_at': 1786000009,
            'body': 'digest',
          }),
          200,
        );
      });

      final event = await api.appendTaskEvent(
        'task-1',
        action: TaskEventAction.summaryPersisted,
        body: 'digest',
      );

      expect(captured.url, Uri.parse('$_baseUrl/api/tasks/task-1/events'));
      expect(jsonDecode(captured.body), {
        'action': 'summary_persisted',
        'body': 'digest',
      });
      expect(event.isSummary, isTrue);
    });

    test('surfaces the relay message for a second summary', () async {
      // `idx_task_events_one_summary_per_task` rejects the second write; the
      // relay maps it to a 400 the user can act on, not a retryable 500.
      final api = apiWith(
        (request) async => http.Response(
          jsonEncode(const {
            'error': 'task task-1 already has a persisted summary',
          }),
          400,
        ),
      );

      await expectLater(
        api.appendTaskEvent(
          'task-1',
          action: TaskEventAction.summaryPersisted,
          body: 'digest',
        ),
        throwsA(
          isA<TaskApiException>()
              .having((e) => e.statusCode, 'statusCode', 400)
              .having(
                (e) => e.toString(),
                'toString',
                'task task-1 already has a persisted summary',
              ),
        ),
      );
    });
  });

  group('updateTask', () {
    test('sends only the fields being changed', () async {
      late http.Request captured;
      final api = apiWith((request) async {
        captured = request;
        return http.Response(jsonEncode(_taskJson()), 200);
      });

      await api.updateTask('task-1', status: TaskStatus.done);

      expect(captured.method, 'PATCH');
      expect(jsonDecode(captured.body), {'status': 'done'});
    });
  });

  group('error handling', () {
    test('falls back to the status code when there is no message', () async {
      final api = apiWith((request) async => http.Response('', 503));
      await expectLater(
        api.createTask(title: 'Ship it'),
        throwsA(
          isA<TaskApiException>().having(
            (e) => e.toString(),
            'toString',
            'HTTP 503',
          ),
        ),
      );
    });

    test('reports an unreadable body instead of a decode crash', () async {
      final api = apiWith((request) async => http.Response('<html>', 502));
      await expectLater(
        api.createTask(title: 'Ship it'),
        throwsA(
          isA<TaskApiException>().having(
            (e) => e.toString(),
            'toString',
            'The relay returned an unreadable task response.',
          ),
        ),
      );
    });
  });

  group('canSign', () {
    test('is false without a signing key', () {
      expect(
        apiWith((_) async => http.Response('{}', 200), signingKey: '').canSign,
        isFalse,
      );
      expect(
        TasksApi(
          httpClient: http_testing.MockClient(
            (_) async => http.Response('{}', 200),
          ),
          baseUrl: _baseUrl,
          nsec: null,
        ).canSign,
        isFalse,
      );
    });

    test('is true with one', () {
      expect(apiWith((_) async => http.Response('{}', 200)).canSign, isTrue);
    });
  });
}
