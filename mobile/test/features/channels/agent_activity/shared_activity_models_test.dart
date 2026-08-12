import 'dart:convert';

import 'package:buzz/features/channels/agent_activity/shared_activity_models.dart';
import 'package:flutter_test/flutter_test.dart';

Map<String, Object?> activity({
  String activityId = '63ca9483-c457-4b24-88de-1f14fa97c499',
  String occurredAt = '2026-08-12T08:39:49Z',
  String activityClass = 'turn',
  String status = 'started',
  String? toolKind,
  int? durationMs,
  Map<String, Object?>? usage,
}) => {
  'activityId': activityId,
  'occurredAt': occurredAt,
  'activityClass': activityClass,
  'status': status,
  'toolKind': ?toolKind,
  'durationMs': ?durationMs,
  'usage': ?usage,
};

String frame(List<Map<String, Object?>> activities) =>
    jsonEncode({'version': 1, 'activities': activities});

void main() {
  group('parseSharedActivityFrame', () {
    test('accepts the closed safe schema', () {
      final items = parseSharedActivityFrame(
        frame([
          activity(),
          activity(
            activityId: 'dd55208d-05a9-41d1-8199-d57664885212',
            activityClass: 'tool',
            status: 'completed',
            toolKind: 'search',
            durationMs: 325,
          ),
          activity(
            activityId: '684d0d5f-aacc-4670-9b63-72ecf805fa0d',
            activityClass: 'usage',
            status: 'completed',
            usage: {'inputTokens': 100, 'outputTokens': 25, 'totalTokens': 125},
          ),
        ]),
      );

      expect(items, hasLength(3));
      expect(items[0].activityClass, SharedActivityClass.turn);
      expect(items[1].toolKind, SharedActivityToolKind.search);
      expect(items[1].durationMs, 325);
      expect(items[2].usage?.totalTokens, 125);
    });

    test('rejects unknown and sensitive fields instead of ignoring them', () {
      for (final sensitiveField in [
        'prompt',
        'message',
        'title',
        'arguments',
        'result',
        'error',
        'model',
        'provider',
        'cost',
        'sessionId',
      ]) {
        final hostile = activity(
          activityClass: 'tool',
          status: 'running',
          toolKind: 'execute',
        )..[sensitiveField] = 'PRIVATE_VALUE';
        expect(
          () => parseSharedActivityFrame(frame([hostile])),
          throwsFormatException,
          reason: sensitiveField,
        );
      }

      expect(
        () => parseSharedActivityFrame(
          jsonEncode({
            'version': 1,
            'activities': [activity()],
            'unknown': true,
          }),
        ),
        throwsFormatException,
      );
    });

    test('rejects unknown variants and class-incompatible fields', () {
      final invalid = <Map<String, Object?>>[
        activity(activityClass: 'message'),
        activity(status: 'thinking'),
        activity(activityClass: 'tool', status: 'running'),
        activity(activityClass: 'turn', status: 'running', toolKind: 'search'),
        activity(
          activityClass: 'usage',
          status: 'running',
          usage: {'totalTokens': 1},
        ),
        activity(
          activityClass: 'usage',
          status: 'completed',
          usage: <String, Object?>{},
        ),
      ];

      for (final candidate in invalid) {
        expect(
          () => parseSharedActivityFrame(frame([candidate])),
          throwsFormatException,
          reason: jsonEncode(candidate),
        );
      }
    });

    test('rejects explicit null for every optional field', () {
      for (final field in ['toolKind', 'durationMs', 'usage']) {
        final candidate = activity()..[field] = null;
        expect(
          () => parseSharedActivityFrame(frame([candidate])),
          throwsFormatException,
          reason: field,
        );
      }
      for (final field in [
        'inputTokens',
        'outputTokens',
        'totalTokens',
        'cacheReadTokens',
        'cacheWriteTokens',
      ]) {
        final candidate = activity(
          activityClass: 'usage',
          status: 'completed',
          usage: {field: null},
        );
        expect(
          () => parseSharedActivityFrame(frame([candidate])),
          throwsFormatException,
          reason: field,
        );
      }
    });

    test('enforces version, byte, count, duration, and usage bounds', () {
      expect(
        () => parseSharedActivityFrame(
          jsonEncode({
            'version': 2,
            'activities': [activity()],
          }),
        ),
        throwsFormatException,
      );
      expect(() => parseSharedActivityFrame(frame([])), throwsFormatException);
      expect(
        () => parseSharedActivityFrame(
          frame(
            List.generate(
              33,
              (index) => activity(
                activityId:
                    '00000000-0000-4000-8000-${index.toString().padLeft(12, '0')}',
              ),
            ),
          ),
        ),
        throwsFormatException,
      );
      expect(
        () => parseSharedActivityFrame(
          frame([activity(status: 'completed', durationMs: 604800001)]),
        ),
        throwsFormatException,
      );
      expect(
        () => parseSharedActivityFrame(
          frame([
            activity(
              activityClass: 'usage',
              status: 'completed',
              usage: {'totalTokens': 1000000000001},
            ),
          ]),
        ),
        throwsFormatException,
      );
      expect(
        () => parseSharedActivityFrame(
          jsonEncode({
            'version': 1,
            'activities': [activity()],
            'padding': List.filled(4096, 'x').join(),
          }),
        ),
        throwsFormatException,
      );
    });
  });

  test('store replaces duplicate activity ids and retains a bounded tail', () {
    final store = SharedActivityStore(maxItems: 3);
    final first = parseSharedActivityFrame(frame([activity()])).single;
    final updated = parseSharedActivityFrame(
      frame([activity(status: 'completed', durationMs: 10)]),
    ).single;

    store.addAll([first]);
    store.addAll([updated]);
    expect(store.items, hasLength(1));
    expect(store.items.single.status, SharedActivityStatus.completed);

    store.addAll([
      for (var index = 1; index <= 4; index++)
        parseSharedActivityFrame(
          frame([
            activity(
              activityId:
                  '00000000-0000-4000-8000-${index.toString().padLeft(12, '0')}',
              occurredAt:
                  '2026-08-12T08:39:${(49 + index).toString().padLeft(2, '0')}Z',
            ),
          ]),
        ).single,
    ]);

    expect(store.items, hasLength(3));
    expect(store.items.map((item) => item.activityId), [
      '00000000-0000-4000-8000-000000000002',
      '00000000-0000-4000-8000-000000000003',
      '00000000-0000-4000-8000-000000000004',
    ]);
  });
}
