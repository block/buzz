import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';

import '../../../shared/utils/string_utils.dart';
import 'active_agent_turns.dart';

const _agentLiveUpdatesChannel = MethodChannel('buzz/agent_live_updates');
const _liveUpdateSafetyLifetime = Duration(hours: 8);

/// Platform-neutral content rendered by Android and iOS live updates.
@immutable
class AgentLiveUpdateContent {
  final String title;
  final String body;
  final int activeAgentCount;
  final DateTime startedAt;
  final DateTime lastActivityAt;
  final DateTime expiresAt;
  final String channelId;
  final String? messageId;

  const AgentLiveUpdateContent({
    required this.title,
    required this.body,
    required this.activeAgentCount,
    required this.startedAt,
    required this.lastActivityAt,
    required this.expiresAt,
    required this.channelId,
    this.messageId,
  });

  Map<String, Object> toPlatformArguments() => {
    'title': title,
    'body': body,
    'activeCount': activeAgentCount,
    'startedAtMillis': startedAt.millisecondsSinceEpoch,
    'lastActivityAtMillis': lastActivityAt.millisecondsSinceEpoch,
    'expiresAtMillis': expiresAt.millisecondsSinceEpoch,
    'channelId': channelId,
    'messageId': ?messageId,
  };

  @override
  bool operator ==(Object other) =>
      identical(this, other) ||
      other is AgentLiveUpdateContent &&
          title == other.title &&
          body == other.body &&
          activeAgentCount == other.activeAgentCount &&
          startedAt == other.startedAt &&
          lastActivityAt == other.lastActivityAt &&
          expiresAt == other.expiresAt &&
          channelId == other.channelId &&
          messageId == other.messageId;

  @override
  int get hashCode => Object.hash(
    title,
    body,
    activeAgentCount,
    startedAt,
    lastActivityAt,
    expiresAt,
    channelId,
    messageId,
  );
}

/// Builds one aggregate live update for the supplied active agent turns.
AgentLiveUpdateContent? buildAgentLiveUpdateContent(
  List<ActiveAgentTurn> turns,
  Map<String, String> channelNames, [
  Map<String, String> agentNames = const {},
]) {
  if (turns.isEmpty) return null;

  final agentPubkeys = turns.map((turn) => turn.agentPubkey).toSet().toList()
    ..sort(
      (a, b) =>
          _agentLabel(a, agentNames).compareTo(_agentLabel(b, agentNames)),
    );
  final agentLabels = agentPubkeys
      .map((pubkey) => _agentLabel(pubkey, agentNames))
      .toList();
  final agentCount = agentPubkeys.length;
  final activeChannelIds = turns.map((turn) => turn.channelId).toSet();
  final activeChannelLabels =
      activeChannelIds
          .map((id) => '#${channelNames[id] ?? 'channel'}')
          .toSet()
          .toList()
        ..sort();
  final startedAt = turns
      .map((turn) => turn.startedAt)
      .reduce((earliest, value) => value.isBefore(earliest) ? value : earliest);
  final lastActivityAt = turns
      .map((turn) => turn.lastActivityAt)
      .reduce((latest, value) => value.isAfter(latest) ? value : latest);
  final livenessExpiresAt = turns
      .map((turn) => turn.lastActivityAt.add(turn.livenessTimeout))
      .reduce((latest, value) => value.isAfter(latest) ? value : latest);
  final safetyExpiresAt = lastActivityAt.add(_liveUpdateSafetyLifetime);
  final tapTarget = [...turns]
    ..sort((a, b) => b.lastActivityAt.compareTo(a.lastActivityAt));
  final targetWithMessage = tapTarget
      .where((turn) => turn.triggeringEventId != null)
      .firstOrNull;
  final target = targetWithMessage ?? tapTarget.first;

  return AgentLiveUpdateContent(
    title: _agentTitle(agentLabels),
    body: _channelSummary(activeChannelLabels),
    activeAgentCount: agentCount,
    startedAt: startedAt,
    lastActivityAt: lastActivityAt,
    expiresAt: livenessExpiresAt.isAfter(safetyExpiresAt)
        ? livenessExpiresAt
        : safetyExpiresAt,
    channelId: target.channelId,
    messageId: target.triggeringEventId,
  );
}

String _agentLabel(String pubkey, Map<String, String> agentNames) {
  final name = agentNames[pubkey]?.trim();
  return name?.isNotEmpty == true ? name! : shortPubkey(pubkey);
}

String _agentTitle(List<String> agentLabels) => switch (agentLabels.length) {
  1 => '${agentLabels.first} is working',
  2 => '${agentLabels.first} and ${agentLabels.last} are working',
  _ =>
    '${agentLabels.first} and ${agentLabels.length - 1} more agents are working',
};

String _channelSummary(
  List<String> channelLabels,
) => switch (channelLabels.length) {
  1 => 'In ${channelLabels.first}',
  2 => 'Across ${channelLabels.first} and ${channelLabels.last}',
  _ =>
    'Across ${channelLabels.take(2).join(', ')} and ${channelLabels.length - 2} more',
};

/// Shows, updates, or dismisses the native agent live update.
Future<void> syncAgentLiveUpdate(AgentLiveUpdateContent? content) async {
  if (kIsWeb ||
      (defaultTargetPlatform != TargetPlatform.android &&
          defaultTargetPlatform != TargetPlatform.iOS)) {
    return;
  }

  try {
    if (content == null) {
      await _agentLiveUpdatesChannel.invokeMethod<void>('dismiss');
      return;
    }
    await _agentLiveUpdatesChannel.invokeMethod<bool>(
      'show',
      content.toPlatformArguments(),
    );
  } on PlatformException catch (error) {
    debugPrint('Unable to update agent live activity: ${error.message}');
  } on MissingPluginException {
    debugPrint('Agent live activity is unavailable in this build.');
  }
}

/// Serializes native live-update calls so their final state matches Dart.
class AgentLiveUpdateSynchronizer {
  Future<void> _pending = Future.value();

  Future<void> sync(AgentLiveUpdateContent? content) {
    final operation = _pending.then((_) => syncAgentLiveUpdate(content));
    _pending = operation.then<void>(
      (_) {},
      onError: (Object error, StackTrace stackTrace) {
        debugPrint('Unable to serialize agent live activity updates: $error');
      },
    );
    return operation;
  }
}
