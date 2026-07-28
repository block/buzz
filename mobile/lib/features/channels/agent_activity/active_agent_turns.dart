import 'package:flutter/foundation.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../../shared/relay/relay.dart';
import 'observer_models.dart';
import 'observer_subscription.dart';

const _activeTurnLivenessTimeout = Duration(seconds: 30);
const _activeTurnClockInterval = Duration(seconds: 5);
const _maximumLivenessInterval = Duration(hours: 24);
const _livenessTimeoutSlack = Duration(seconds: 30);

/// An agent turn reconstructed from observer frames and still considered live.
@immutable
class ActiveAgentTurn {
  final String agentPubkey;
  final String channelId;
  final String turnId;
  final DateTime startedAt;
  final DateTime lastActivityAt;
  final Duration livenessTimeout;
  final String? triggeringEventId;

  const ActiveAgentTurn({
    required this.agentPubkey,
    required this.channelId,
    required this.turnId,
    required this.startedAt,
    required this.lastActivityAt,
    this.livenessTimeout = _activeTurnLivenessTimeout,
    this.triggeringEventId,
  });

  /// Returns this turn with an updated locally observed activity time.
  ActiveAgentTurn copyWith({
    required DateTime lastActivityAt,
    Duration? livenessTimeout,
  }) => ActiveAgentTurn(
    agentPubkey: agentPubkey,
    channelId: channelId,
    turnId: turnId,
    startedAt: startedAt,
    lastActivityAt: lastActivityAt,
    livenessTimeout: livenessTimeout ?? this.livenessTimeout,
    triggeringEventId: triggeringEventId,
  );
}

/// Reconstructs live turns from replay and live frames, pruning stale turns.
List<ActiveAgentTurn> reduceActiveAgentTurns(
  Map<String, List<ObserverFrame>> framesByAgent, {
  required DateTime now,
}) {
  final activeTurns = <ActiveAgentTurn>[];

  for (final entry in framesByAgent.entries) {
    final agentPubkey = entry.key.toLowerCase();
    final frames = [...entry.value]..sort(_compareFrames);
    final turnsById = <String, ActiveAgentTurn>{};
    final terminalAtById = <String, DateTime>{};

    for (final frame in frames) {
      final frameOrderAt = _frameTimestamp(frame);
      final frameAt = _localFrameTimestamp(frame);
      switch (frame.kind) {
        case 'turn_started':
          final channelId = frame.channelId;
          if (channelId == null) continue;
          final turnId = frame.turnId ?? 'seq-${frame.seq}';
          final startedAt = _safeStartedAt(frame, frameAt);
          turnsById[turnId] = ActiveAgentTurn(
            agentPubkey: agentPubkey,
            channelId: channelId,
            turnId: turnId,
            startedAt: startedAt,
            lastActivityAt: frameAt,
            livenessTimeout: _livenessTimeout(frame.payload),
            triggeringEventId: _triggeringEventId(frame.payload),
          );
        case 'turn_completed':
        case 'turn_error':
        case 'agent_panic':
          final turnId = frame.turnId;
          if (turnId != null) {
            terminalAtById[turnId] = frameOrderAt;
            turnsById.remove(turnId);
            continue;
          }
          final channelId = frame.channelId;
          if (channelId == null) continue;
          final matchingTurn = turnsById.values
              .where((turn) => turn.channelId == channelId)
              .firstOrNull;
          if (matchingTurn != null) {
            terminalAtById[matchingTurn.turnId] = frameOrderAt;
            turnsById.remove(matchingTurn.turnId);
          }
        case 'acp_read':
        case 'acp_write':
        case 'turn_liveness':
          final turnId = frame.turnId;
          if (turnId == null) continue;
          final activeTurn = turnsById[turnId];
          if (activeTurn != null) {
            turnsById[turnId] = activeTurn.copyWith(
              lastActivityAt: frameAt,
              livenessTimeout: frame.kind == 'turn_liveness'
                  ? _livenessTimeout(frame.payload)
                  : null,
            );
            continue;
          }

          final channelId = frame.channelId;
          if (channelId == null) continue;
          final terminalAt = terminalAtById[turnId];
          if (terminalAt != null && !frameOrderAt.isAfter(terminalAt)) continue;
          turnsById[turnId] = ActiveAgentTurn(
            agentPubkey: agentPubkey,
            channelId: channelId,
            turnId: turnId,
            startedAt: _safeStartedAt(frame, frameAt),
            lastActivityAt: frameAt,
            livenessTimeout: _livenessTimeout(frame.payload),
          );
      }
    }

    activeTurns.addAll(
      turnsById.values.where(
        (turn) => now.difference(turn.lastActivityAt) <= turn.livenessTimeout,
      ),
    );
  }

  activeTurns.sort((a, b) {
    final started = a.startedAt.compareTo(b.startedAt);
    if (started != 0) return started;
    final agent = a.agentPubkey.compareTo(b.agentPubkey);
    if (agent != 0) return agent;
    return a.turnId.compareTo(b.turnId);
  });
  return List.unmodifiable(activeTurns);
}

final _activeAgentTurnClockProvider = StreamProvider<DateTime>((ref) async* {
  yield DateTime.now().toUtc();
  yield* Stream<DateTime>.periodic(
    _activeTurnClockInterval,
    (_) => DateTime.now().toUtc(),
  );
});

final activeAgentTurnsProvider = Provider<List<ActiveAgentTurn>>((ref) {
  final observerState = ref.watch(observerRelayProvider);
  ref.watch(appLifecycleProvider);
  final now =
      ref.watch(_activeAgentTurnClockProvider).value ?? DateTime.now().toUtc();
  return reduceActiveAgentTurns(observerState.framesByAgent, now: now);
});

DateTime _frameTimestamp(ObserverFrame frame) =>
    DateTime.tryParse(frame.timestamp) ??
    DateTime.fromMillisecondsSinceEpoch(frame.seq);

DateTime _localFrameTimestamp(ObserverFrame frame) =>
    frame.receivedAt ?? _frameTimestamp(frame);

DateTime _safeStartedAt(ObserverFrame frame, DateTime frameAt) {
  final hostFrameAt = DateTime.tryParse(frame.timestamp);
  final hostStartedAt = DateTime.tryParse(frame.startedAt ?? '');
  if (hostFrameAt == null ||
      hostStartedAt == null ||
      hostStartedAt.isAfter(hostFrameAt)) {
    return frameAt;
  }
  return frameAt.subtract(hostFrameAt.difference(hostStartedAt));
}

String? _triggeringEventId(dynamic payload) {
  if (payload is! Map) return null;
  final eventIds = payload['triggeringEventIds'];
  if (eventIds is! List) return null;
  for (final eventId in eventIds) {
    if (eventId is String && eventId.isNotEmpty) return eventId;
  }
  return null;
}

Duration _livenessTimeout(dynamic payload) {
  final rawInterval = payload is Map ? payload['livenessIntervalSecs'] : null;
  if (rawInterval is! num) return _activeTurnLivenessTimeout;

  final intervalSeconds = rawInterval.toInt();
  if (intervalSeconds <= 0) {
    return _maximumLivenessInterval + _livenessTimeoutSlack;
  }
  final boundedInterval = intervalSeconds.clamp(
    5,
    _maximumLivenessInterval.inSeconds,
  );
  final timeoutSeconds = boundedInterval + _livenessTimeoutSlack.inSeconds;
  return Duration(
    seconds: timeoutSeconds < _activeTurnLivenessTimeout.inSeconds
        ? _activeTurnLivenessTimeout.inSeconds
        : timeoutSeconds,
  );
}

int _compareFrames(ObserverFrame a, ObserverFrame b) {
  final timestamp = _frameTimestamp(a).compareTo(_frameTimestamp(b));
  return timestamp != 0 ? timestamp : a.seq.compareTo(b.seq);
}
