import 'package:flutter_test/flutter_test.dart';
import 'package:buzz/shared/agent_usage/agent_usage_models.dart';

void main() {
  group('AgentTurnMetricPayload.fromJson', () {
    test('parses a minimal payload with only required fields', () {
      final payload = AgentTurnMetricPayload.fromJson({
        'harness': 'goose',
        'timestamp': '2026-07-01T20:11:03.213Z',
      });
      expect(payload.harness, 'goose');
      expect(payload.model, isNull);
      expect(payload.contextUsedTokens, isNull);
      expect(payload.accountUsageWindows, isEmpty);
    });

    test('throws when harness is missing', () {
      expect(
        () => AgentTurnMetricPayload.fromJson({
          'timestamp': '2026-07-01T20:11:03Z',
        }),
        throwsFormatException,
      );
    });

    test('throws when timestamp is missing or unparseable', () {
      expect(
        () => AgentTurnMetricPayload.fromJson({'harness': 'goose'}),
        throwsFormatException,
      );
      expect(
        () => AgentTurnMetricPayload.fromJson({
          'harness': 'goose',
          'timestamp': 'not-a-date',
        }),
        throwsFormatException,
      );
    });

    test('ignores unknown fields (forward compatibility)', () {
      final payload = AgentTurnMetricPayload.fromJson({
        'harness': 'goose',
        'timestamp': '2026-07-01T20:11:03Z',
        'somethingFromTheFuture': {'nested': true},
      });
      expect(payload.harness, 'goose');
    });

    test('parses context window snapshot and account usage windows', () {
      final payload = AgentTurnMetricPayload.fromJson({
        'harness': 'buzz-agent',
        'timestamp': '2026-07-01T20:11:03Z',
        'contextUsedTokens': 15392,
        'contextLimitTokens': 272000,
        'accountUsageWindows': [
          {
            'label': 'Session',
            'usedPercent': 12.5,
            'resetAt': '2026-07-02T01:00:00Z',
          },
          {'label': 'Weekly', 'usedPercent': 41.0},
        ],
      });
      expect(payload.contextUsedTokens, 15392);
      expect(payload.contextLimitTokens, 272000);
      expect(payload.accountUsageWindows, hasLength(2));
      expect(payload.accountUsageWindows.first.label, 'Session');
      expect(payload.accountUsageWindows.first.resetAt, isNotNull);
      expect(payload.accountUsageWindows.last.resetAt, isNull);
    });

    test('rejects negative or fractional token counts', () {
      for (final value in [-1, 1.5]) {
        expect(
          () => AgentTurnMetricPayload.fromJson({
            'harness': 'goose',
            'timestamp': '2026-07-01T20:11:03Z',
            'contextUsedTokens': value,
          }),
          throwsFormatException,
        );
      }
      expect(
        () => AgentTurnMetricPayload.fromJson({
          'harness': 'goose',
          'timestamp': '2026-07-01T20:11:03Z',
          'cumulative': {'inputTokens': -1},
        }),
        throwsFormatException,
      );
    });

    test('rejects malformed usage-window shapes and reset times', () {
      for (final windows in [
        'not-an-array',
        ['not-an-object'],
        [
          {
            'label': 'Session',
            'usedPercent': 12,
            'resetAt': 'not-rfc3339',
          },
        ],
      ]) {
        expect(
          () => AgentTurnMetricPayload.fromJson({
            'harness': 'goose',
            'timestamp': '2026-07-01T20:11:03Z',
            'accountUsageWindows': windows,
          }),
          throwsFormatException,
        );
      }
    });

    test(
      'rejects a non-finite or negative accountUsageWindows.usedPercent',
      () {
        expect(
          () => AgentTurnMetricPayload.fromJson({
            'harness': 'goose',
            'timestamp': '2026-07-01T20:11:03Z',
            'accountUsageWindows': [
              {'label': 'Session', 'usedPercent': -5.0},
            ],
          }),
          throwsFormatException,
        );
      },
    );

    test('rejects a negative cumulative.costUsd', () {
      expect(
        () => AgentTurnMetricPayload.fromJson({
          'harness': 'goose',
          'timestamp': '2026-07-01T20:11:03Z',
          'cumulative': {'costUsd': -0.5},
        }),
        throwsFormatException,
      );
    });

    test('parses cumulative token counts', () {
      final payload = AgentTurnMetricPayload.fromJson({
        'harness': 'goose',
        'timestamp': '2026-07-01T20:11:03Z',
        'cumulative': {
          'inputTokens': 45210,
          'outputTokens': 9876,
          'totalTokens': 55086,
          'costUsd': 0.41,
        },
      });
      expect(payload.cumulative?.totalTokens, 55086);
      expect(payload.cumulative?.costUsd, 0.41);
    });
  });

  group('mergeAgentUsageSnapshot', () {
    AgentTurnMetricPayload payloadAt(
      String timestamp, {
      int? contextUsedTokens,
      int? contextLimitTokens,
      List<AgentUsageWindow> accountUsageWindows = const [],
      AgentTokenCounts? cumulative,
      String harness = 'goose',
      String? model,
    }) {
      return AgentTurnMetricPayload(
        harness: harness,
        model: model,
        timestamp: DateTime.parse(timestamp),
        contextUsedTokens: contextUsedTokens,
        contextLimitTokens: contextLimitTokens,
        accountUsageWindows: accountUsageWindows,
        cumulative: cumulative,
      );
    }

    test('first event seeds every field', () {
      final snapshot = mergeAgentUsageSnapshot(
        null,
        payloadAt(
          '2026-07-01T10:00:00Z',
          contextUsedTokens: 1000,
          contextLimitTokens: 2000,
          accountUsageWindows: const [
            AgentUsageWindow(label: 'Session', usedPercent: 10),
          ],
        ),
      );
      expect(snapshot.contextUsedTokens, 1000);
      expect(snapshot.accountUsageWindows.single.label, 'Session');
      expect(snapshot.lastEventAt, DateTime.parse('2026-07-01T10:00:00Z'));
    });

    test(
      'a later event that omits a field keeps the previous value for that field',
      () {
        final first = mergeAgentUsageSnapshot(
          null,
          payloadAt(
            '2026-07-01T10:00:00Z',
            accountUsageWindows: const [
              AgentUsageWindow(label: 'Session', usedPercent: 10),
            ],
          ),
        );

        // Second turn reports a fresh context snapshot but no usage windows
        // this time — the windows must not be wiped out.
        final second = mergeAgentUsageSnapshot(
          first,
          payloadAt(
            '2026-07-01T10:05:00Z',
            contextUsedTokens: 500,
            contextLimitTokens: 1000,
          ),
        );

        expect(second.contextUsedTokens, 500);
        expect(second.accountUsageWindows.single.label, 'Session');
        expect(
          second.accountUsageWindowsAt,
          DateTime.parse('2026-07-01T10:00:00Z'),
        );
        expect(second.lastEventAt, DateTime.parse('2026-07-01T10:05:00Z'));
      },
    );

    test('an out-of-order (older) event does not clobber newer field data', () {
      final first = mergeAgentUsageSnapshot(
        null,
        payloadAt(
          '2026-07-01T10:05:00Z',
          contextUsedTokens: 500,
          contextLimitTokens: 1000,
        ),
      );

      // A stale/backfilled event for an earlier turn arrives after the fact.
      final second = mergeAgentUsageSnapshot(
        first,
        payloadAt(
          '2026-07-01T10:00:00Z',
          contextUsedTokens: 999999,
          contextLimitTokens: 1000000,
        ),
      );

      expect(second.contextUsedTokens, 500);
      expect(second.contextLimitTokens, 1000);
      // lastEventAt tracks the newest event seen overall, regardless of order.
      expect(second.lastEventAt, DateTime.parse('2026-07-01T10:05:00Z'));
    });

    test('accountUsageWindows replaces the whole list, not per-label', () {
      final first = mergeAgentUsageSnapshot(
        null,
        payloadAt(
          '2026-07-01T10:00:00Z',
          accountUsageWindows: const [
            AgentUsageWindow(label: 'Session', usedPercent: 10),
            AgentUsageWindow(label: 'Weekly', usedPercent: 20),
          ],
        ),
      );

      final second = mergeAgentUsageSnapshot(
        first,
        payloadAt(
          '2026-07-01T10:05:00Z',
          accountUsageWindows: const [
            AgentUsageWindow(label: 'Session', usedPercent: 15),
          ],
        ),
      );

      // The newer snapshot is authoritative wholesale — "Weekly" is gone, not
      // merged forward, matching NIP-AM's "last-write-wins display snapshot".
      expect(second.accountUsageWindows, hasLength(1));
      expect(second.accountUsageWindows.single.usedPercent, 15);
    });

    test('model persists once reported even if a later event omits it', () {
      final first = mergeAgentUsageSnapshot(
        null,
        payloadAt('2026-07-01T10:00:00Z', model: 'claude-sonnet-4-5'),
      );
      final second = mergeAgentUsageSnapshot(
        first,
        payloadAt('2026-07-01T10:05:00Z'),
      );
      expect(second.model, 'claude-sonnet-4-5');
    });
  });

  group('agentUsageStatusFor', () {
    test('unknown when there is no snapshot and no subscription error', () {
      expect(
        agentUsageStatusFor(null, subscriptionErrored: false),
        AgentUsageStatus.unknown,
      );
    });

    test(
      'error when there is no snapshot and the subscription is unhealthy',
      () {
        expect(
          agentUsageStatusFor(null, subscriptionErrored: true),
          AgentUsageStatus.error,
        );
      },
    );

    test(
      'fresh when the snapshot is recent, regardless of subscription health',
      () {
        final snapshot = AgentUsageSnapshot(
          harness: 'goose',
          lastEventAt: DateTime.parse('2026-07-01T10:00:00Z'),
        );
        expect(
          agentUsageStatusFor(
            snapshot,
            subscriptionErrored: true,
            now: DateTime.parse('2026-07-01T10:05:00Z'),
          ),
          AgentUsageStatus.fresh,
        );
      },
    );

    test('stale once the snapshot exceeds the freshness window', () {
      final snapshot = AgentUsageSnapshot(
        harness: 'goose',
        lastEventAt: DateTime.parse('2026-07-01T00:00:00Z'),
      );
      expect(
        agentUsageStatusFor(
          snapshot,
          subscriptionErrored: false,
          now: DateTime.parse('2026-07-01T10:00:00Z'),
        ),
        AgentUsageStatus.stale,
      );
    });

    test('uses the shared 45 minute freshness window', () {
      final snapshot = AgentUsageSnapshot(
        harness: 'goose',
        lastEventAt: DateTime.parse('2026-07-01T10:00:00Z'),
      );
      expect(
        agentUsageStatusFor(
          snapshot,
          subscriptionErrored: false,
          now: DateTime.parse('2026-07-01T10:45:00Z'),
        ),
        AgentUsageStatus.fresh,
      );
      expect(
        agentUsageStatusFor(
          snapshot,
          subscriptionErrored: false,
          now: DateTime.parse('2026-07-01T10:45:01Z'),
        ),
        AgentUsageStatus.stale,
      );
    });
  });

  group('primaryUsageFraction', () {
    test('null when there is no snapshot', () {
      expect(primaryUsageFraction(null), isNull);
    });

    test(
      'falls back to context window fraction when there are no quota windows',
      () {
        final snapshot = AgentUsageSnapshot(
          harness: 'goose',
          lastEventAt: DateTime.now(),
          contextUsedTokens: 50,
          contextLimitTokens: 100,
        );
        expect(primaryUsageFraction(snapshot), 0.5);
      },
    );

    test('prefers the highest account usage window over context usage', () {
      final snapshot = AgentUsageSnapshot(
        harness: 'goose',
        lastEventAt: DateTime.now(),
        contextUsedTokens: 10,
        contextLimitTokens: 100,
        accountUsageWindows: const [
          AgentUsageWindow(label: 'Session', usedPercent: 30),
          AgentUsageWindow(label: 'Weekly', usedPercent: 87),
        ],
      );
      expect(primaryUsageFraction(snapshot), closeTo(0.87, 0.0001));
    });

    test('clamps a context fraction that exceeds the reported limit', () {
      final snapshot = AgentUsageSnapshot(
        harness: 'goose',
        lastEventAt: DateTime.now(),
        contextUsedTokens: 150,
        contextLimitTokens: 100,
      );
      expect(primaryUsageFraction(snapshot), 1.0);
    });
  });
}
