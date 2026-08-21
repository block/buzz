import 'package:buzz/shared/tasks/task.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  group('buildCreateTaskPayload', () {
    test('sends only the fields that were set', () {
      expect(buildCreateTaskPayload(title: 'Ship the relay change'), {
        'title': 'Ship the relay change',
        'source': 'mobile',
      });
    });

    test('trims the title and drops a whitespace-only body', () {
      expect(buildCreateTaskPayload(title: '  ship it  ', body: '   \n  '), {
        'title': 'ship it',
        'source': 'mobile',
      });
    });

    test('includes every optional field when provided', () {
      final payload = buildCreateTaskPayload(
        title: 'Fix the migration',
        body: '  needs a backfill  ',
        channelId: '11111111-1111-1111-1111-111111111111',
        sourceRef: 'abc123',
        assignee: 'ab' * 32,
        priority: 3,
        dueAt: DateTime.utc(2026, 8, 24, 13),
        source: 'claude',
      );

      expect(payload, {
        'title': 'Fix the migration',
        'body': 'needs a backfill',
        'channel_id': '11111111-1111-1111-1111-111111111111',
        'source_ref': 'abc123',
        'assignee': 'ab' * 32,
        'priority': 3,
        'due_at': '2026-08-24T13:00:00.000Z',
        'source': 'claude',
      });
    });

    test('serializes a local due date as UTC RFC 3339', () {
      // The relay deserializes `due_at` into a `DateTime<Utc>`, so a local
      // instant must be converted rather than sent with an offset the handler
      // would have to interpret.
      final local = DateTime.utc(2026, 8, 24, 13).toLocal();
      expect(
        buildCreateTaskPayload(title: 'due', dueAt: local)['due_at'],
        '2026-08-24T13:00:00.000Z',
      );
    });

    test('rejects a blank title before the request is made', () {
      expect(
        () => buildCreateTaskPayload(title: '   '),
        throwsA(isA<ArgumentError>()),
      );
      expect(
        () => buildCreateTaskPayload(title: ''),
        throwsA(isA<ArgumentError>()),
      );
    });

    test('counts title length in characters, not UTF-16 code units', () {
      // `validate_title` uses `chars().count()` and Postgres `length()` counts
      // characters, so a 200-character multi-byte title is legal even though
      // `String.length` reports more than 200.
      final multibyte = 'é' * maxTaskTitleChars;
      expect(multibyte.length, maxTaskTitleChars);
      expect(taskTitleLength(multibyte), maxTaskTitleChars);
      expect(buildCreateTaskPayload(title: multibyte)['title'], multibyte);

      final emoji = '🐝' * maxTaskTitleChars;
      expect(emoji.length, maxTaskTitleChars * 2, reason: 'surrogate pairs');
      expect(taskTitleLength(emoji), maxTaskTitleChars);
      expect(buildCreateTaskPayload(title: emoji)['title'], emoji);
    });

    test('rejects a title one character over the relay ceiling', () {
      expect(
        () => buildCreateTaskPayload(title: 'a' * (maxTaskTitleChars + 1)),
        throwsA(isA<ArgumentError>()),
      );
      expect(
        () => buildCreateTaskPayload(title: '🐝' * (maxTaskTitleChars + 1)),
        throwsA(isA<ArgumentError>()),
      );
    });
  });

  group('buildTaskEventPayload', () {
    test('emits the relay action string', () {
      expect(
        buildTaskEventPayload(
          action: TaskEventAction.summaryPersisted,
          body: '  ## Thread summary  ',
        ),
        {'action': 'summary_persisted', 'body': '## Thread summary'},
      );
      expect(
        buildTaskEventPayload(
          action: TaskEventAction.commented,
          body: 'looks good',
        )['action'],
        'commented',
      );
    });

    test('rejects a blank body the relay would 400', () {
      expect(
        () => buildTaskEventPayload(
          action: TaskEventAction.commented,
          body: '  ',
        ),
        throwsA(isA<ArgumentError>()),
      );
    });
  });

  group('composeTaskBody', () {
    test('returns null when there is nothing to send', () {
      expect(composeTaskBody(), isNull);
      expect(composeTaskBody(body: '   '), isNull);
      expect(composeTaskBody(agentHandles: const ['  ']), isNull);
    });

    test('leads with the mention line, then the body', () {
      expect(
        composeTaskBody(body: ' do the thing ', agentHandles: const ['Ada']),
        '@Ada\n\ndo the thing',
      );
    });

    test('sends mentions alone when no body was typed', () {
      expect(
        composeTaskBody(agentHandles: const ['Ada', 'Grace']),
        '@Ada @Grace',
      );
    });
  });

  group('resolveAgentHandles', () {
    // Literal 64-char hex: `'aa' * 32` is not a constant expression, and
    // these are used inside const map literals below.
    const ada =
        'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa';
    const grace =
        'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb';
    const unknown =
        'cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc';

    test('prefers a profile name, then the directory, then the pubkey', () {
      expect(
        resolveAgentHandles(
          agentPubkeys: const [ada, grace, unknown],
          profileNames: const {ada: 'Ada'},
          directoryNames: const {grace: 'Grace'},
        ),
        ['Ada', 'cccccccc', 'Grace'],
      );
    });

    test('is sorted, so an unordered Set produces a stable body', () {
      // `channelBotPubkeysProvider` hands back a Set; iteration order there is
      // not part of its contract.
      final handles = resolveAgentHandles(
        agentPubkeys: {grace, ada},
        profileNames: const {ada: 'Ada', grace: 'grace'},
        directoryNames: const {},
      );
      expect(handles, ['Ada', 'grace']);
    });

    test('normalizes lookup keys to lowercase and ignores blank names', () {
      expect(
        resolveAgentHandles(
          agentPubkeys: [ada.toUpperCase()],
          profileNames: const {ada: '   '},
          directoryNames: const {ada: 'Directory Ada'},
        ),
        ['Directory Ada'],
      );
    });

    test('deduplicates agents that share a display name', () {
      expect(
        resolveAgentHandles(
          agentPubkeys: const [ada, grace],
          profileNames: const {ada: 'Claude', grace: 'Claude'},
          directoryNames: const {},
        ),
        ['Claude'],
      );
    });
  });

  group('Task.fromJson', () {
    test('decodes the relay wire shape', () {
      final task = Task.fromJson({
        'id': 'task-1',
        'channel_id': 'channel-1',
        'created_by': 'ab' * 32,
        'assignee': null,
        'parent_task_id': null,
        'title': 'Ship it',
        'body': 'with a flag',
        'status': 'in_progress',
        'priority': 2,
        'source': 'mobile',
        'source_ref': 'event-1',
        'due_at': 1787000000,
        'done_at': null,
        'archived_at': null,
        'created_at': 1786000000,
        'updated_at': 1786000001,
      });

      expect(task.id, 'task-1');
      expect(task.status, TaskStatus.inProgress);
      expect(task.priority, 2);
      expect(task.assignee, isNull);
      expect(task.sourceRef, 'event-1');
      // Inbound timestamps are Unix seconds, not the RFC 3339 the client sends.
      expect(
        task.dueAt,
        DateTime.fromMillisecondsSinceEpoch(1787000000 * 1000, isUtc: true),
      );
      expect(task.doneAt, isNull);
    });

    test('degrades an unknown status instead of throwing', () {
      // A client built before a status was added must still render the task.
      expect(TaskStatus.fromWire('deferred'), TaskStatus.todo);
      expect(TaskStatus.fromWire(null), TaskStatus.todo);
    });

    test('rejects a response missing an id or title', () {
      expect(
        () => Task.fromJson({'title': 'no id'}),
        throwsA(isA<FormatException>()),
      );
    });
  });

  group('TaskDetail', () {
    TaskEvent event(int id, String action) => TaskEvent.fromJson({
      'id': id,
      'task_id': 'task-1',
      'action': action,
      'created_at': 1786000000 + id,
      'body': action,
    });

    final task = Task.fromJson({
      'id': 'task-1',
      'title': 'Ship it',
      'status': 'todo',
      'priority': 0,
      'created_at': 1786000000,
      'updated_at': 1786000000,
    });

    test('finds the single persisted summary', () {
      final detail = TaskDetail(
        task: task,
        events: [
          event(1, 'created'),
          event(2, 'commented'),
          event(3, 'summary_persisted'),
        ],
      );
      expect(detail.summary?.id, 3);
      expect(detail.summary?.isSummary, isTrue);
    });

    test('reports no summary when the history has none', () {
      final detail = TaskDetail(task: task, events: [event(1, 'created')]);
      expect(detail.summary, isNull);
    });

    test('keeps an unrecognised action as text', () {
      // `task_events.action` is unconstrained TEXT so a new harness can write
      // an action this build has never heard of.
      expect(event(9, 'handed_off').action, 'handed_off');
      expect(event(9, 'handed_off').isSummary, isFalse);
    });
  });
}
