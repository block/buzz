import 'package:flutter_test/flutter_test.dart';
import 'package:buzz/features/channels/agent_activity/active_agent_turns.dart';
import 'package:buzz/features/channels/agent_activity/observer_models.dart';

void main() {
  test('tracks an active turn and refreshes it from liveness', () {
    final turns = reduceActiveAgentTurns({
      'AGENT-A': [
        _frame(
          seq: 1,
          second: 1,
          kind: 'turn_started',
          payload: {
            'triggeringEventIds': ['message-1'],
          },
        ),
        _frame(seq: 2, second: 11, kind: 'turn_liveness'),
      ],
    }, now: DateTime.utc(2026, 7, 27, 12, 0, 20));

    expect(turns, hasLength(1));
    expect(turns.single.agentPubkey, 'agent-a');
    expect(turns.single.channelId, 'channel-1');
    expect(turns.single.turnId, 'turn-1');
    expect(turns.single.triggeringEventId, 'message-1');
    expect(turns.single.startedAt, DateTime.utc(2026, 7, 27, 12, 0, 1));
    expect(turns.single.lastActivityAt, DateTime.utc(2026, 7, 27, 12, 0, 11));
  });

  test('removes a turn after a terminal frame', () {
    final turns = reduceActiveAgentTurns({
      'agent-a': [
        _frame(seq: 1, second: 1, kind: 'turn_started'),
        _frame(seq: 2, second: 2, kind: 'turn_completed'),
      ],
    }, now: DateTime.utc(2026, 7, 27, 12, 0, 20));

    expect(turns, isEmpty);
  });

  test('recovers a live turn when its start frame was missed', () {
    final turns = reduceActiveAgentTurns({
      'agent-a': [
        _frame(
          seq: 9,
          second: 30,
          kind: 'turn_liveness',
          startedAt: DateTime.utc(2026, 7, 27, 12).toIso8601String(),
        ),
      ],
    }, now: DateTime.utc(2026, 7, 27, 12, 0, 40));

    expect(turns, hasLength(1));
    expect(turns.single.startedAt, DateTime.utc(2026, 7, 27, 12));
    expect(turns.single.lastActivityAt, DateTime.utc(2026, 7, 27, 12, 0, 30));
  });

  test('tracks concurrent turns independently', () {
    final turns = reduceActiveAgentTurns({
      'agent-a': [_frame(seq: 1, second: 1, kind: 'turn_started')],
      'agent-b': [
        _frame(
          seq: 1,
          second: 2,
          kind: 'turn_started',
          turnId: 'turn-2',
          channelId: 'channel-2',
        ),
      ],
    }, now: DateTime.utc(2026, 7, 27, 12, 0, 20));

    expect(turns.map((turn) => turn.agentPubkey), ['agent-a', 'agent-b']);
    expect(turns.map((turn) => turn.channelId), ['channel-1', 'channel-2']);
  });

  test('expires a turn when its terminal frame was missed', () {
    final turns = reduceActiveAgentTurns({
      'agent-a': [
        _frame(seq: 1, second: 1, kind: 'turn_started'),
        _frame(seq: 2, second: 11, kind: 'turn_liveness'),
      ],
    }, now: DateTime.utc(2026, 7, 27, 12, 0, 42));

    expect(turns, isEmpty);
  });

  test('uses the harness liveness interval when pruning a quiet turn', () {
    final frame = _frame(
      seq: 1,
      second: 1,
      kind: 'turn_started',
      payload: {'livenessIntervalSecs': 60},
    );

    final stillActive = reduceActiveAgentTurns({
      'agent-a': [frame],
    }, now: DateTime.utc(2026, 7, 27, 12, 1, 1));
    final expired = reduceActiveAgentTurns({
      'agent-a': [frame],
    }, now: DateTime.utc(2026, 7, 27, 12, 1, 32));

    expect(stillActive, hasLength(1));
    expect(stillActive.single.livenessTimeout, const Duration(seconds: 90));
    expect(expired, isEmpty);
  });

  test('translates host timestamps onto the device clock', () {
    final receivedAt = DateTime.utc(2026, 7, 27, 12, 0, 10);
    final turns = reduceActiveAgentTurns({
      'agent-a': [
        ObserverFrame(
          seq: 1,
          timestamp: DateTime.utc(2026, 7, 27, 13, 0, 10).toIso8601String(),
          kind: 'turn_started',
          channelId: 'channel-1',
          turnId: 'turn-1',
          startedAt: DateTime.utc(2026, 7, 27, 13).toIso8601String(),
          receivedAt: receivedAt,
        ),
      ],
    }, now: receivedAt);

    expect(turns, hasLength(1));
    expect(turns.single.startedAt, DateTime.utc(2026, 7, 27, 12));
    expect(turns.single.lastActivityAt, receivedAt);
  });
}

ObserverFrame _frame({
  required int seq,
  required int second,
  required String kind,
  String turnId = 'turn-1',
  String channelId = 'channel-1',
  String? startedAt,
  dynamic payload = const <String, dynamic>{},
}) {
  return ObserverFrame(
    seq: seq,
    timestamp: DateTime.utc(2026, 7, 27, 12, 0, second).toIso8601String(),
    kind: kind,
    channelId: channelId,
    turnId: turnId,
    startedAt: startedAt,
    payload: payload,
  );
}
