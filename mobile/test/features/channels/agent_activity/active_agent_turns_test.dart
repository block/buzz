import 'package:flutter_test/flutter_test.dart';
import 'package:buzz/features/channels/agent_activity/active_agent_turns.dart';
import 'package:buzz/features/channels/agent_activity/observer_models.dart';

void main() {
  test('tracks and refreshes a live turn using device receipt time', () {
    final turns = reduceAgentTurnStates({
      'AGENT-A': [
        _frame(
          seq: 1,
          second: 1,
          kind: 'turn_started',
          threadHeadId: 'thread-1',
          receivedSecond: 10,
          payload: {
            'triggeringEventIds': ['message-1'],
          },
        ),
        _frame(seq: 2, second: 11, kind: 'turn_liveness', receivedSecond: 20),
      ],
    }, now: DateTime.utc(2026, 8, 16, 12, 0, 25));

    expect(turns, hasLength(1));
    expect(turns.single.agentPubkey, 'agent-a');
    expect(turns.single.phase, AgentTurnPhase.working);
    expect(turns.single.threadHeadId, 'thread-1');
    expect(turns.single.triggeringEventId, 'message-1');
    expect(turns.single.lastActivityAt, DateTime.utc(2026, 8, 16, 12, 0, 20));
  });

  test('rescopes a continuing turn after a successful native steer', () {
    final turns = reduceAgentTurnStates({
      'agent-a': [
        _frame(
          seq: 1,
          second: 1,
          kind: 'turn_started',
          threadHeadId: 'thread-a',
          receivedSecond: 1,
          payload: {
            'triggeringEventIds': ['message-a'],
          },
        ),
        _frame(
          seq: 2,
          second: 2,
          kind: 'turn_rescoped',
          threadHeadId: 'thread-b',
          receivedSecond: 2,
          payload: {'triggeringEventId': 'message-b'},
        ),
      ],
    }, now: DateTime.utc(2026, 8, 16, 12, 0, 3));

    expect(turns.single.threadHeadId, 'thread-b');
    expect(turns.single.triggeringEventId, 'message-b');
    expect(turns.single.turnId, 'turn-1');
    expect(turns.single.isWorking, isTrue);
  });

  test(
    'terminal and activity frames carry a rescope without a rescope event',
    () {
      final terminal = reduceAgentTurnStates({
        'agent-a': [
          _frame(
            seq: 1,
            second: 1,
            kind: 'turn_started',
            threadHeadId: 'thread-a',
          ),
          _frame(
            seq: 2,
            second: 2,
            kind: 'turn_completed',
            threadHeadId: 'thread-b',
          ),
        ],
      }, now: DateTime.utc(2026, 8, 16, 12, 0, 3));

      expect(terminal.single.threadHeadId, 'thread-b');
      expect(terminal.single.phase, AgentTurnPhase.finished);

      final working = reduceAgentTurnStates({
        'agent-a': [
          _frame(
            seq: 1,
            second: 1,
            kind: 'turn_started',
            threadHeadId: 'thread-a',
          ),
          _frame(
            seq: 2,
            second: 2,
            kind: 'turn_liveness',
            threadHeadId: 'thread-b',
          ),
        ],
      }, now: DateTime.utc(2026, 8, 16, 12, 0, 3));

      expect(working.single.threadHeadId, 'thread-b');
      expect(working.single.isWorking, isTrue);
    },
  );

  test(
    'explicit null scope moves later activity and terminal frames to root',
    () {
      List<ObserverFrame> frames(String kind) => [
        _frame(
          seq: 1,
          second: 1,
          kind: 'turn_started',
          threadHeadId: 'thread-a',
        ),
        _frame(seq: 2, second: 2, kind: kind, hasThreadScope: true),
      ];

      final working = reduceAgentTurnStates({
        'agent-a': frames('turn_liveness'),
      }, now: DateTime.utc(2026, 8, 16, 12, 0, 3));
      expect(working.single.threadHeadId, isNull);
      expect(working.single.isWorking, isTrue);

      final terminal = reduceAgentTurnStates({
        'agent-a': frames('turn_completed'),
      }, now: DateTime.utc(2026, 8, 16, 12, 0, 3));
      expect(terminal.single.threadHeadId, isNull);
      expect(terminal.single.phase, AgentTurnPhase.finished);
    },
  );

  test('legacy scope omission preserves the latest known thread', () {
    final turns = reduceAgentTurnStates({
      'agent-a': [
        _frame(
          seq: 1,
          second: 1,
          kind: 'turn_started',
          threadHeadId: 'thread-a',
        ),
        _frame(seq: 2, second: 2, kind: 'turn_liveness'),
      ],
    }, now: DateTime.utc(2026, 8, 16, 12, 0, 3));

    expect(turns.single.threadHeadId, 'thread-a');
  });

  test('explicit rescope event moves a turn from a thread to root', () {
    final turns = reduceAgentTurnStates({
      'agent-a': [
        _frame(
          seq: 1,
          second: 1,
          kind: 'turn_started',
          threadHeadId: 'thread-a',
        ),
        _frame(seq: 2, second: 2, kind: 'turn_rescoped'),
        _frame(seq: 3, second: 3, kind: 'turn_liveness'),
      ],
    }, now: DateTime.utc(2026, 8, 16, 12, 0, 4));

    expect(turns.single.threadHeadId, isNull);
    expect(turns.single.isWorking, isTrue);
  });

  test('preserves explicit completion and error outcomes', () {
    final turns = reduceAgentTurnStates({
      'agent-a': [
        _frame(seq: 1, second: 1, kind: 'turn_started'),
        _frame(seq: 2, second: 2, kind: 'turn_completed'),
      ],
      'agent-b': [
        _frame(seq: 1, second: 1, kind: 'turn_started', turnId: 'turn-b'),
        _frame(
          seq: 2,
          second: 2,
          kind: 'turn_error',
          turnId: 'turn-b',
          payload: {'error': 'Tool permission denied'},
        ),
      ],
    }, now: DateTime.utc(2026, 8, 16, 12, 1));

    expect(turns, hasLength(2));
    expect(turns[0].phase, AgentTurnPhase.finished);
    expect(turns[1].phase, AgentTurnPhase.error);
    expect(turns[1].errorMessage, 'Tool permission denied');
  });

  test('orders a turn by sequence when the host clock moves backward', () {
    final turns = reduceAgentTurnStates({
      'agent-a': [
        _frame(
          seq: 1,
          second: 10,
          kind: 'turn_started',
          sessionId: 'session-1',
          receivedSecond: 1,
        ),
        _frame(
          seq: 2,
          second: 1,
          kind: 'turn_completed',
          sessionId: 'session-1',
          receivedSecond: 2,
        ),
      ],
    }, now: DateTime.utc(2026, 8, 16, 12, 0, 20));

    expect(turns, hasLength(1));
    expect(turns.single.phase, AgentTurnPhase.finished);
    expect(turns.single.lastActivityAt, DateTime.utc(2026, 8, 16, 12, 0, 2));
  });

  test('orders a turn by sequence when the host clock jumps forward', () {
    final turns = reduceAgentTurnStates({
      'agent-a': [
        _frame(
          seq: 1,
          second: 1,
          kind: 'turn_started',
          sessionId: 'session-1',
          receivedSecond: 1,
        ),
        _frame(
          seq: 2,
          second: 50,
          kind: 'acp_read',
          sessionId: 'session-1',
          receivedSecond: 2,
        ),
        _frame(
          seq: 3,
          second: 3,
          kind: 'turn_completed',
          sessionId: 'session-1',
          receivedSecond: 3,
        ),
      ],
    }, now: DateTime.utc(2026, 8, 16, 12, 0, 20));

    expect(turns, hasLength(1));
    expect(turns.single.phase, AgentTurnPhase.finished);
    expect(turns.single.lastActivityAt, DateTime.utc(2026, 8, 16, 12, 0, 3));
  });

  test('reports cancelled completion separately from a finished turn', () {
    final turns = reduceAgentTurnStates({
      'agent-a': [
        _frame(seq: 1, second: 1, kind: 'turn_started'),
        _frame(
          seq: 2,
          second: 2,
          kind: 'turn_completed',
          payload: {'outcome': 'cancelled'},
        ),
      ],
    }, now: DateTime.utc(2026, 8, 16, 12, 1));

    expect(turns, hasLength(1));
    expect(turns.single.phase, AgentTurnPhase.cancelled);
  });

  test('keeps an error terminal when generic completion arrives later', () {
    final turns = reduceAgentTurnStates({
      'agent-a': [
        _frame(seq: 1, second: 1, kind: 'turn_started'),
        _frame(
          seq: 2,
          second: 2,
          kind: 'turn_error',
          payload: {'error': 'Agent timed out'},
        ),
        _frame(seq: 3, second: 3, kind: 'turn_completed'),
      ],
    }, now: DateTime.utc(2026, 8, 16, 12, 1));

    expect(turns, hasLength(1));
    expect(turns.single.phase, AgentTurnPhase.error);
    expect(turns.single.errorMessage, 'Agent timed out');
    expect(turns.single.terminalAt, DateTime.utc(2026, 8, 16, 12, 0, 2));
  });

  test('expires silence without claiming the turn finished', () {
    final turns = reduceAgentTurnStates({
      'agent-a': [_frame(seq: 1, second: 1, kind: 'turn_started')],
    }, now: DateTime.utc(2026, 8, 16, 12, 0, 32));

    expect(turns, isEmpty);
  });

  test('uses the advertised liveness interval to expire quiet turns', () {
    final frames = {
      'agent-a': [
        _frame(
          seq: 1,
          second: 1,
          kind: 'turn_started',
          payload: {'livenessIntervalSecs': 120},
        ),
      ],
    };

    final beforeTimeout = reduceAgentTurnStates(
      frames,
      now: DateTime.utc(2026, 8, 16, 12, 2, 30),
    );
    final afterTimeout = reduceAgentTurnStates(
      frames,
      now: DateTime.utc(2026, 8, 16, 12, 2, 32),
    );

    expect(beforeTimeout, hasLength(1));
    expect(beforeTimeout.single.livenessTimeout, const Duration(seconds: 150));
    expect(afterTimeout, isEmpty);
  });

  test('honors advertised liveness intervals longer than one day', () {
    final frames = {
      'agent-a': [
        _frame(
          seq: 1,
          second: 1,
          kind: 'turn_started',
          payload: {'livenessIntervalSecs': 48 * 60 * 60},
        ),
      ],
    };

    final beforeTimeout = reduceAgentTurnStates(
      frames,
      now: DateTime.utc(2026, 8, 18, 12, 0, 30),
    );
    final afterTimeout = reduceAgentTurnStates(
      frames,
      now: DateTime.utc(2026, 8, 18, 12, 0, 32),
    );

    expect(beforeTimeout, hasLength(1));
    expect(
      beforeTimeout.single.livenessTimeout,
      const Duration(hours: 48, seconds: 30),
    );
    expect(afterTimeout, isEmpty);
  });

  test('keeps liveness-disabled turns until the bounded crash backstop', () {
    final frames = {
      'agent-a': [
        _frame(
          seq: 1,
          second: 1,
          kind: 'turn_started',
          payload: {'livenessIntervalSecs': 0},
        ),
      ],
    };

    final longRunning = reduceAgentTurnStates(
      frames,
      now: DateTime.utc(2026, 8, 23, 12),
    );
    final pastBackstop = reduceAgentTurnStates(
      frames,
      now: DateTime.utc(2026, 8, 23, 12, 0, 32),
    );

    expect(longRunning, hasLength(1));
    expect(
      longRunning.single.livenessTimeout,
      const Duration(days: 7, seconds: 30),
    );
    expect(pastBackstop, isEmpty);
  });

  test('does not assume a 30-second cadence when joining mid-turn', () {
    final frames = {
      'agent-a': [
        _frame(
          seq: 1,
          second: const Duration(days: 6).inSeconds + 1,
          kind: 'acp_read',
          startedAt: DateTime.utc(2026, 8, 16, 12).toIso8601String(),
        ),
      ],
    };

    final afterLegacyTimeout = reduceAgentTurnStates(
      frames,
      now: DateTime.utc(2026, 8, 22, 12, 0, 32),
    );
    final pastBackstop = reduceAgentTurnStates(
      frames,
      now: DateTime.utc(2026, 8, 23, 12, 0, 32),
    );

    expect(afterLegacyTimeout, hasLength(1));
    expect(
      afterLegacyTimeout.single.livenessTimeout,
      const Duration(days: 7, seconds: 30),
    );
    expect(pastBackstop, isEmpty);
  });

  test('uses a liveness frame cadence when joining mid-turn', () {
    final turns = reduceAgentTurnStates({
      'agent-a': [
        _frame(
          seq: 1,
          second: 1,
          kind: 'turn_liveness',
          payload: {'livenessIntervalSecs': 120},
        ),
      ],
    }, now: DateTime.utc(2026, 8, 16, 12, 2, 30));

    expect(turns, hasLength(1));
    expect(turns.single.livenessTimeout, const Duration(seconds: 150));
  });

  test('recovers a missed start and rejects stale post-terminal liveness', () {
    final turns = reduceAgentTurnStates({
      'agent-a': [
        _frame(
          seq: 1,
          second: 1,
          kind: 'turn_liveness',
          startedAt: DateTime.utc(2026, 8, 16, 11, 59).toIso8601String(),
        ),
        _frame(seq: 2, second: 2, kind: 'turn_completed'),
        _frame(
          seq: 0,
          second: 1,
          kind: 'turn_liveness',
          startedAt: DateTime.utc(2026, 8, 16, 11, 59).toIso8601String(),
        ),
      ],
    }, now: DateTime.utc(2026, 8, 16, 12, 0, 20));

    expect(turns, hasLength(1));
    expect(turns.single.phase, AgentTurnPhase.finished);
    expect(turns.single.startedAt, DateTime.utc(2026, 8, 16, 11, 59));
  });

  test('terminal without a turn id updates the latest turn in its channel', () {
    final turns = reduceAgentTurnStates({
      'agent-a': [
        _frame(seq: 1, second: 1, kind: 'turn_started'),
        _frame(
          seq: 2,
          second: 2,
          kind: 'agent_panic',
          turnId: null,
          payload: {'error': 'Process exited'},
        ),
      ],
    }, now: DateTime.utc(2026, 8, 16, 12, 0, 20));

    expect(turns.single.phase, AgentTurnPhase.error);
    expect(turns.single.errorMessage, 'Process exited');
  });

  test('terminal without a turn id stays within its observer thread scope', () {
    final turns = reduceAgentTurnStates({
      'agent-a': [
        _frame(
          seq: 1,
          second: 1,
          kind: 'turn_started',
          turnId: 'turn-a',
          threadHeadId: 'thread-a',
        ),
        _frame(
          seq: 2,
          second: 2,
          kind: 'turn_started',
          turnId: 'turn-b',
          threadHeadId: 'thread-b',
        ),
        _frame(
          seq: 3,
          second: 3,
          kind: 'agent_panic',
          turnId: null,
          threadHeadId: 'thread-b',
          payload: {'error': 'Process exited'},
        ),
      ],
    }, now: DateTime.utc(2026, 8, 16, 12, 0, 20));

    expect(
      turns.singleWhere((turn) => turn.turnId == 'turn-a').phase,
      AgentTurnPhase.working,
    );
    expect(
      turns.singleWhere((turn) => turn.turnId == 'turn-b').phase,
      AgentTurnPhase.error,
    );
  });

  test(
    'retains terminal outcomes beside the composer for a bounded window',
    () {
      final terminalAt = DateTime.utc(2026, 8, 16, 12, 0, 2);
      final states = [
        AgentTurnState(
          agentPubkey: 'agent-a',
          channelId: 'channel-1',
          turnId: 'turn-a',
          startedAt: DateTime.utc(2026, 8, 16, 12),
          lastActivityAt: terminalAt,
          livenessTimeout: const Duration(seconds: 30),
          phase: AgentTurnPhase.error,
          terminalAt: terminalAt,
          errorMessage: 'Agent timed out',
        ),
      ];

      expect(
        composerAgentTurnStates(
          states,
          now: terminalAt.add(const Duration(seconds: 30)),
        ),
        hasLength(1),
      );
      expect(
        composerAgentTurnStates(
          states,
          now: terminalAt.add(const Duration(seconds: 31)),
        ),
        isEmpty,
      );
    },
  );
}

ObserverFrame _frame({
  required int seq,
  required int second,
  required String kind,
  String? turnId = 'turn-1',
  String channelId = 'channel-1',
  String? threadHeadId,
  bool? hasThreadScope,
  String? sessionId,
  int? receivedSecond,
  String? startedAt,
  dynamic payload = const <String, dynamic>{},
}) {
  return ObserverFrame(
    seq: seq,
    timestamp: DateTime.utc(2026, 8, 16, 12, 0, second).toIso8601String(),
    kind: kind,
    channelId: channelId,
    threadHeadId: threadHeadId,
    hasThreadScope: hasThreadScope,
    sessionId: sessionId,
    turnId: turnId,
    startedAt: startedAt,
    receivedAt: receivedSecond == null
        ? null
        : DateTime.utc(2026, 8, 16, 12, 0, receivedSecond),
    payload: payload,
  );
}
