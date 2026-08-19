import 'package:flutter/foundation.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../../shared/mentions/agent_identity_provider.dart';
import '../../../shared/profile/user_cache_provider.dart';
import '../channel_management_provider.dart';
import '../channel_typing_provider.dart';
import 'active_agent_turns.dart';
import 'observer_subscription.dart';

const _observerLivenessSlack = Duration(seconds: 30);
const _maximumTypingSupersedeGrace = Duration(seconds: 30);

/// Composer activity is scoped either to a channel or to one thread.
typedef ComposerActivityKey = ({String channelId, String? threadHeadId});

enum AgentWorkingSource { observer, typing }

/// One agent currently surfaced beside the composer.
@immutable
class WorkingAgentSignal {
  final String pubkey;
  final AgentWorkingSource source;
  final bool canViewActivity;

  /// Whether this signal represents live work rather than a retained outcome.
  final bool isWorking;

  /// Known observer phase; typing-only signals have no lifecycle phase yet.
  final AgentTurnPhase? phase;
  final String? turnId;
  final DateTime? startedAt;

  const WorkingAgentSignal({
    required this.pubkey,
    required this.source,
    required this.canViewActivity,
    this.isWorking = true,
    this.phase,
    this.turnId,
    this.startedAt,
  });
}

/// Agent work and human typing are intentionally separate presentation lanes.
@immutable
class ComposerActivityState {
  final List<WorkingAgentSignal> agents;
  final List<TypingEntry> humanTyping;

  const ComposerActivityState({
    required this.agents,
    required this.humanTyping,
  });
}

/// Unified composer state. Scoped observer activity is authoritative;
/// kind:20002 typing fills gaps, including for legacy observer frames that do
/// not carry a thread id.
final composerActivityStateProvider = Provider.autoDispose
    .family<ComposerActivityState, ComposerActivityKey>((ref, key) {
      final currentPubkey = ref.watch(currentPubkeyProvider)?.toLowerCase();
      final typing = [
        for (final entry in ref.watch(channelTypingProvider(key.channelId)))
          if (entry.threadHeadId == key.threadHeadId &&
              entry.pubkey.toLowerCase() != currentPubkey)
            entry,
      ];
      final profiles = ref.watch(userCacheProvider);
      final channelMembers =
          ref.watch(channelMembersProvider(key.channelId)).asData?.value ??
          const <ChannelMember>[];
      final channelAgents = <String>{
        ...ref.watch(agentMentionPubkeysProvider(key.channelId)),
        for (final member in channelMembers)
          if (member.isBot) member.pubkey.toLowerCase(),
        for (final entry in profiles.entries)
          if (entry.value.ownerPubkey != null) entry.key.toLowerCase(),
      };
      final observerState = ref.watch(observerRelayProvider);
      final ownerByAgent =
          ref.watch(agentOwnersProvider).asData?.value ??
          const <String, String>{};
      final composerTurns = ref.watch(composerAgentTurnStatesProvider);
      final activeByAgent = <String, AgentTurnState>{};
      for (final turn in composerTurns) {
        if (turn.channelId != key.channelId ||
            turn.threadHeadId != key.threadHeadId) {
          continue;
        }
        final existing = activeByAgent[turn.agentPubkey];
        if (existing == null ||
            _compareComposerTurnRecency(turn, existing) > 0) {
          activeByAgent[turn.agentPubkey] = turn;
        }
      }

      bool canView(String pubkey) {
        final normalized = pubkey.toLowerCase();
        // A frame reaches this state only after the subscription validated its
        // agent signature, owner p-tag, and NIP-44 decryption. That is stronger
        // local authorization evidence than eventually-consistent profile data.
        if (observerState.framesByAgent.containsKey(normalized)) return true;
        if (currentPubkey == null) return false;
        return ownerByAgent[normalized]?.toLowerCase() == currentPubkey ||
            profiles[normalized]?.ownerPubkey?.toLowerCase() == currentPubkey;
      }

      final signals = <String, WorkingAgentSignal>{};
      for (final entry in activeByAgent.entries) {
        final hasValidatedObserverFrame = observerState.framesByAgent
            .containsKey(entry.key);
        if (!channelAgents.contains(entry.key) && !hasValidatedObserverFrame) {
          continue;
        }
        final turn = entry.value;
        if (!turn.isWorking && !canView(entry.key)) continue;
        signals[entry.key] = WorkingAgentSignal(
          pubkey: entry.key,
          source: AgentWorkingSource.observer,
          canViewActivity: canView(entry.key),
          isWorking: turn.isWorking,
          phase: turn.phase,
          turnId: turn.turnId,
          startedAt: turn.startedAt,
        );
      }

      final humans = <TypingEntry>[];
      for (final entry in typing) {
        final pubkey = entry.pubkey.toLowerCase();
        if (!channelAgents.contains(pubkey) &&
            !observerState.framesByAgent.containsKey(pubkey)) {
          humans.add(entry);
          continue;
        }
        final turn = activeByAgent[pubkey];
        final supersedesWorkingTurn =
            turn != null &&
            turn.isWorking &&
            entry.turnId != turn.turnId &&
            _typingSupersedesWorkingTurn(entry, turn);
        if (signals[pubkey]?.isWorking == true && !supersedesWorkingTurn) {
          continue;
        }
        final liveTurn = turn?.isWorking == true && !supersedesWorkingTurn
            ? turn
            : null;
        signals[pubkey] = WorkingAgentSignal(
          pubkey: pubkey,
          source: AgentWorkingSource.typing,
          canViewActivity: canView(pubkey),
          isWorking: true,
          phase: liveTurn?.phase,
          turnId: liveTurn?.turnId,
          startedAt: liveTurn?.startedAt,
        );
      }

      final agents = signals.values.toList()
        ..sort((a, b) {
          final aAt = a.startedAt;
          final bAt = b.startedAt;
          if (aAt != null && bAt != null) return aAt.compareTo(bAt);
          if (aAt != null) return -1;
          if (bAt != null) return 1;
          return a.pubkey.compareTo(b.pubkey);
        });
      return ComposerActivityState(
        agents: List.unmodifiable(agents),
        humanTyping: List.unmodifiable(humans),
      );
    });

/// Agent pubkeys working in a channel, for the members badge and sheet.
final workingBotPubkeysProvider = Provider.autoDispose
    .family<Set<String>, String>((ref, channelId) {
      final threadScopes = <String?>{
        null,
        for (final entry in ref.watch(channelTypingProvider(channelId)))
          entry.threadHeadId,
        for (final turn in ref.watch(composerAgentTurnStatesProvider))
          if (turn.channelId == channelId) turn.threadHeadId,
      };
      final working = <String>{};
      for (final threadHeadId in threadScopes) {
        final activity = ref.watch(
          composerActivityStateProvider((
            channelId: channelId,
            threadHeadId: threadHeadId,
          )),
        );
        working.addAll(
          activity.agents
              .where((agent) => agent.isWorking)
              .map((agent) => agent.pubkey),
        );
      }
      return Set.unmodifiable(working);
    });

int _compareComposerTurnRecency(AgentTurnState a, AgentTurnState b) {
  final activity = a.lastActivityAt.compareTo(b.lastActivityAt);
  if (activity != 0) return activity;
  final started = a.startedAt.compareTo(b.startedAt);
  if (started != 0) return started;
  if (a.isWorking != b.isWorking) return a.isWorking ? 1 : -1;
  return a.turnId.compareTo(b.turnId);
}

bool _typingSupersedesWorkingTurn(TypingEntry typing, AgentTurnState turn) {
  final advertisedCadenceMs =
      turn.livenessTimeout.inMilliseconds -
      _observerLivenessSlack.inMilliseconds;
  final graceMs = advertisedCadenceMs <= 0
      ? _maximumTypingSupersedeGrace.inMilliseconds
      : advertisedCadenceMs
            .clamp(
              TypingEntry.ttl.inMilliseconds,
              _maximumTypingSupersedeGrace.inMilliseconds,
            )
            .toInt();
  return typing.receivedAt.isAfter(
    turn.lastActivityAt.add(Duration(milliseconds: graceMs)),
  );
}
