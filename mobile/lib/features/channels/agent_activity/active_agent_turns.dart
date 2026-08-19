import 'package:flutter/foundation.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../../shared/relay/relay.dart';
import 'observer_models.dart';
import 'observer_subscription.dart';

const _defaultLivenessTimeout = Duration(seconds: 30);
const _activeTurnClockInterval = Duration(seconds: 5);
const _terminalComposerRetention = Duration(seconds: 30);
// Matches buzz-acp's MAX_TURN_DURATION_CEILING_SECS. A disabled or unusually
// sparse liveness cadence must not expire before any legal turn can finish.
const _maximumTurnDuration = Duration(days: 7);
const _livenessTimeoutSlack = Duration(seconds: 30);

/// Lifecycle state reconstructed from owner-scoped observer frames.
enum AgentTurnPhase { working, finished, cancelled, error }

/// One observed agent turn, including its explicit terminal outcome when known.
@immutable
class AgentTurnState {
  final String agentPubkey;
  final String channelId;
  final String? threadHeadId;
  final String turnId;
  final DateTime startedAt;
  final DateTime lastActivityAt;
  final Duration livenessTimeout;
  final AgentTurnPhase phase;
  final DateTime? terminalAt;
  final String? errorMessage;
  final String? triggeringEventId;

  const AgentTurnState({
    required this.agentPubkey,
    required this.channelId,
    this.threadHeadId,
    required this.turnId,
    required this.startedAt,
    required this.lastActivityAt,
    required this.livenessTimeout,
    required this.phase,
    this.terminalAt,
    this.errorMessage,
    this.triggeringEventId,
  });

  bool get isWorking => phase == AgentTurnPhase.working;

  AgentTurnState withActivity({
    required DateTime at,
    required String? threadHeadId,
    Duration? timeout,
  }) => AgentTurnState(
    agentPubkey: agentPubkey,
    channelId: channelId,
    threadHeadId: threadHeadId,
    turnId: turnId,
    startedAt: startedAt,
    lastActivityAt: at,
    livenessTimeout: timeout ?? livenessTimeout,
    phase: AgentTurnPhase.working,
    triggeringEventId: triggeringEventId,
  );

  AgentTurnState withTerminal({
    required AgentTurnPhase phase,
    required DateTime at,
    required String? threadHeadId,
    String? errorMessage,
  }) => AgentTurnState(
    agentPubkey: agentPubkey,
    channelId: channelId,
    threadHeadId: threadHeadId,
    turnId: turnId,
    startedAt: startedAt,
    lastActivityAt: at,
    livenessTimeout: livenessTimeout,
    phase: phase,
    terminalAt: at,
    errorMessage: errorMessage,
    triggeringEventId: triggeringEventId,
  );
}

/// Reconstructs live and explicitly terminal turns from the retained frame
/// buffer. Silent working turns expire instead of being mislabeled finished.
List<AgentTurnState> reduceAgentTurnStates(
  Map<String, List<ObserverFrame>> framesByAgent, {
  required DateTime now,
}) {
  final states = <AgentTurnState>[];

  for (final entry in framesByAgent.entries) {
    final agentPubkey = entry.key.toLowerCase();
    final frames = orderObserverFrames(entry.value);
    final turnsById = <String, AgentTurnState>{};
    final terminalSequenceById = <String, int>{};

    for (final frame in frames) {
      final frameAt = frame.receivedAt ?? _frameTimestamp(frame);
      switch (frame.kind) {
        case 'turn_started':
          final channelId = frame.channelId;
          if (channelId == null) continue;
          final turnId = frame.turnId ?? 'seq-${frame.seq}';
          turnsById[turnId] = AgentTurnState(
            agentPubkey: agentPubkey,
            channelId: channelId,
            threadHeadId: frame.threadHeadId,
            turnId: turnId,
            startedAt: _safeStartedAt(frame, frameAt),
            lastActivityAt: frameAt,
            livenessTimeout: _livenessTimeout(frame.payload),
            phase: AgentTurnPhase.working,
            triggeringEventId: _triggeringEventId(frame.payload),
          );
        case 'turn_completed':
        case 'turn_error':
        case 'agent_panic':
          final terminalPhase = frame.kind == 'turn_completed'
              ? _completionPhase(frame.payload)
              : AgentTurnPhase.error;
          final turnId = frame.turnId;
          if (turnId != null) {
            terminalSequenceById[turnId] = frame.seq;
            final existing = turnsById[turnId];
            // The harness's generic completion guard can run after its result
            // handler emits the specific failure outcome.
            if (frame.kind == 'turn_completed' &&
                existing?.phase == AgentTurnPhase.error) {
              continue;
            }
            final channelId = existing?.channelId ?? frame.channelId;
            if (channelId == null) continue;
            turnsById[turnId] =
                existing?.withTerminal(
                  phase: terminalPhase,
                  at: frameAt,
                  threadHeadId: frame.hasThreadScope
                      ? frame.threadHeadId
                      : existing.threadHeadId,
                  errorMessage: _turnError(frame.payload),
                ) ??
                AgentTurnState(
                  agentPubkey: agentPubkey,
                  channelId: channelId,
                  threadHeadId: frame.threadHeadId,
                  turnId: turnId,
                  startedAt: _safeStartedAt(frame, frameAt),
                  lastActivityAt: frameAt,
                  livenessTimeout: _livenessTimeout(frame.payload),
                  phase: terminalPhase,
                  terminalAt: frameAt,
                  errorMessage: _turnError(frame.payload),
                );
            continue;
          }

          final channelId = frame.channelId;
          if (channelId == null) continue;
          final matching = turnsById.values
              .where(
                (turn) =>
                    turn.channelId == channelId &&
                    turn.threadHeadId == frame.threadHeadId &&
                    turn.isWorking,
              )
              .fold<AgentTurnState?>(
                null,
                (latest, turn) =>
                    latest == null ||
                        turn.lastActivityAt.isAfter(latest.lastActivityAt)
                    ? turn
                    : latest,
              );
          if (matching == null) continue;
          terminalSequenceById[matching.turnId] = frame.seq;
          turnsById[matching.turnId] = matching.withTerminal(
            phase: terminalPhase,
            at: frameAt,
            threadHeadId: frame.hasThreadScope
                ? frame.threadHeadId
                : matching.threadHeadId,
            errorMessage: _turnError(frame.payload),
          );
        case 'turn_rescoped':
          final turnId = frame.turnId;
          if (turnId == null) continue;
          final existing = turnsById[turnId];
          if (existing == null) continue;
          turnsById[turnId] = AgentTurnState(
            agentPubkey: existing.agentPubkey,
            channelId: existing.channelId,
            threadHeadId: frame.threadHeadId,
            turnId: existing.turnId,
            startedAt: existing.startedAt,
            lastActivityAt: frameAt,
            livenessTimeout: existing.livenessTimeout,
            phase: existing.phase,
            terminalAt: existing.terminalAt,
            errorMessage: existing.errorMessage,
            triggeringEventId:
                _triggeringEventId(frame.payload) ?? existing.triggeringEventId,
          );
        case 'acp_read':
        case 'acp_write':
        case 'turn_liveness':
          final turnId = frame.turnId;
          if (turnId == null) continue;
          final existing = turnsById[turnId];
          if (existing?.isWorking == true) {
            turnsById[turnId] = existing!.withActivity(
              at: frameAt,
              threadHeadId: frame.hasThreadScope
                  ? frame.threadHeadId
                  : existing.threadHeadId,
              timeout: frame.kind == 'turn_liveness'
                  ? _livenessTimeout(frame.payload)
                  : null,
            );
            continue;
          }

          final terminalSequence = terminalSequenceById[turnId];
          if (terminalSequence != null && frame.seq <= terminalSequence) {
            continue;
          }
          final channelId = frame.channelId;
          if (channelId == null) continue;
          turnsById[turnId] = AgentTurnState(
            agentPubkey: agentPubkey,
            channelId: channelId,
            threadHeadId: frame.threadHeadId,
            turnId: turnId,
            startedAt: _safeStartedAt(frame, frameAt),
            lastActivityAt: frameAt,
            // A live-only subscription can join after turn_started. Ordinary
            // ACP frames do not repeat the configured cadence, so absence here
            // means unknown rather than the legacy 30-second default.
            livenessTimeout: _livenessTimeout(
              frame.payload,
              missingFallback: _maximumTurnDuration + _livenessTimeoutSlack,
            ),
            phase: AgentTurnPhase.working,
          );
      }
    }

    states.addAll(
      turnsById.values.where(
        (turn) =>
            !turn.isWorking ||
            (now.difference(turn.lastActivityAt) <= turn.livenessTimeout &&
                !now.isAfter(
                  turn.startedAt.add(
                    _maximumTurnDuration + _livenessTimeoutSlack,
                  ),
                )),
      ),
    );
  }

  states.sort((a, b) {
    final started = a.startedAt.compareTo(b.startedAt);
    if (started != 0) return started;
    final agent = a.agentPubkey.compareTo(b.agentPubkey);
    if (agent != 0) return agent;
    return a.turnId.compareTo(b.turnId);
  });
  return List.unmodifiable(states);
}

/// Most recent state for one agent in one channel.
AgentTurnState? latestAgentTurnState(
  Iterable<AgentTurnState> states, {
  required String agentPubkey,
  required String channelId,
  required String? threadHeadId,
  String? turnId,
}) {
  final normalizedAgent = agentPubkey.toLowerCase();
  AgentTurnState? latest;
  for (final state in states) {
    if (state.agentPubkey != normalizedAgent ||
        state.channelId != channelId ||
        state.threadHeadId != threadHeadId ||
        (turnId != null && state.turnId != turnId)) {
      continue;
    }
    if (latest == null || state.lastActivityAt.isAfter(latest.lastActivityAt)) {
      latest = state;
    }
  }
  return latest;
}

final _activeAgentTurnClockProvider = StreamProvider<DateTime>((ref) async* {
  yield DateTime.now().toUtc();
  yield* Stream<DateTime>.periodic(
    _activeTurnClockInterval,
    (_) => DateTime.now().toUtc(),
  );
});

/// Observer-derived turn states, including buffered terminal outcomes.
final agentTurnStatesProvider = Provider<List<AgentTurnState>>((ref) {
  final observerState = ref.watch(observerRelayProvider);
  if (observerState.framesByAgent.isEmpty) return const [];
  ref.watch(appLifecycleProvider);
  final now =
      ref.watch(_activeAgentTurnClockProvider).value ?? DateTime.now().toUtc();
  return reduceAgentTurnStates(observerState.framesByAgent, now: now);
});

/// Observer-derived turns that are still live according to local receipt time.
final activeAgentTurnsProvider = Provider<List<AgentTurnState>>((ref) {
  return [
    for (final turn in ref.watch(agentTurnStatesProvider))
      if (turn.isWorking) turn,
  ];
});

/// Working turns plus recent explicit outcomes that remain actionable beside
/// the composer for a short, bounded window.
final composerAgentTurnStatesProvider = Provider<List<AgentTurnState>>((ref) {
  final now =
      ref.watch(_activeAgentTurnClockProvider).value ?? DateTime.now().toUtc();
  return composerAgentTurnStates(ref.watch(agentTurnStatesProvider), now: now);
});

/// Filters turn states to those that should remain visible by the composer.
@visibleForTesting
List<AgentTurnState> composerAgentTurnStates(
  Iterable<AgentTurnState> states, {
  required DateTime now,
}) => List.unmodifiable([
  for (final state in states)
    if (state.isWorking ||
        !now.isAfter(
          (state.terminalAt ?? state.lastActivityAt).add(
            _terminalComposerRetention,
          ),
        ))
      state,
]);

DateTime _frameTimestamp(ObserverFrame frame) =>
    DateTime.tryParse(frame.timestamp)?.toUtc() ??
    DateTime.fromMillisecondsSinceEpoch(frame.seq, isUtc: true);

DateTime _safeStartedAt(ObserverFrame frame, DateTime frameAt) {
  final hostFrameAt = DateTime.tryParse(frame.timestamp)?.toUtc();
  final hostStartedAt = DateTime.tryParse(frame.startedAt ?? '')?.toUtc();
  if (hostFrameAt == null ||
      hostStartedAt == null ||
      hostStartedAt.isAfter(hostFrameAt)) {
    return frameAt;
  }
  return frameAt.subtract(hostFrameAt.difference(hostStartedAt));
}

String? _triggeringEventId(dynamic payload) {
  if (payload is! Map) return null;
  final directEventId = payload['triggeringEventId'];
  if (directEventId is String && directEventId.isNotEmpty) {
    return directEventId;
  }
  final eventIds = payload['triggeringEventIds'];
  if (eventIds is! List) return null;
  for (final eventId in eventIds) {
    if (eventId is String && eventId.isNotEmpty) return eventId;
  }
  return null;
}

String? _turnError(dynamic payload) {
  if (payload is! Map) return null;
  final error = payload['error'];
  return error is String && error.trim().isNotEmpty ? error.trim() : null;
}

AgentTurnPhase _completionPhase(dynamic payload) {
  if (payload is Map && payload['outcome'] == 'cancelled') {
    return AgentTurnPhase.cancelled;
  }
  return AgentTurnPhase.finished;
}

Duration _livenessTimeout(
  dynamic payload, {
  Duration missingFallback = _defaultLivenessTimeout,
}) {
  final rawInterval = payload is Map ? payload['livenessIntervalSecs'] : null;
  if (rawInterval is! num) return missingFallback;

  final intervalSeconds = rawInterval.toInt();
  if (intervalSeconds <= 0) {
    return _maximumTurnDuration + _livenessTimeoutSlack;
  }
  final boundedInterval = intervalSeconds.clamp(
    5,
    _maximumTurnDuration.inSeconds,
  );
  final timeoutSeconds = boundedInterval + _livenessTimeoutSlack.inSeconds;
  return Duration(
    seconds: timeoutSeconds < _defaultLivenessTimeout.inSeconds
        ? _defaultLivenessTimeout.inSeconds
        : timeoutSeconds,
  );
}
