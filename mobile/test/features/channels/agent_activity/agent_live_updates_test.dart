import 'dart:async';

import 'package:buzz/features/channels/agent_activity/active_agent_turns.dart';
import 'package:buzz/features/channels/agent_activity/agent_live_updates.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  test('builds a single-agent update with a deep-link target', () {
    final content = buildAgentLiveUpdateContent(
      [_turn(agent: 'agent-a', channel: 'channel-1', message: 'message-1')],
      {'channel-1': 'agents'},
      {'agent-a': 'Codex'},
    );

    expect(content, isNotNull);
    expect(content!.title, 'Codex is working');
    expect(content.body, 'In #agents');
    expect(content.activeAgentCount, 1);
    expect(content.channelId, 'channel-1');
    expect(content.messageId, 'message-1');
    expect(content.expiresAt, DateTime.utc(2026, 7, 27, 20, 0, 10));
    expect(
      content.toPlatformArguments()['expiresAtMillis'],
      content.expiresAt.millisecondsSinceEpoch,
    );
    expect(
      content.toPlatformArguments(),
      isNot(contains('timeoutAfterMillis')),
    );
  });

  test('aggregates multiple agents and channels into one update', () {
    final content = buildAgentLiveUpdateContent(
      [
        _turn(agent: 'agent-a', channel: 'channel-1'),
        _turn(
          agent: 'agent-b',
          channel: 'channel-2',
          minute: 1,
          message: 'message-2',
        ),
        _turn(agent: 'agent-b', channel: 'channel-2', minute: 2),
      ],
      {'channel-1': 'agents', 'channel-2': 'design'},
      {'agent-a': 'Codex', 'agent-b': 'Maya'},
    );

    expect(content, isNotNull);
    expect(content!.title, 'Codex and Maya are working');
    expect(content.body, 'Across #agents and #design');
    expect(content.activeAgentCount, 2);
    expect(content.startedAt, DateTime.utc(2026, 7, 27, 12));
    expect(content.channelId, 'channel-2');
    expect(content.messageId, 'message-2');
  });

  test('returns no content without active turns', () {
    expect(buildAgentLiveUpdateContent(const [], const {}), isNull);
  });

  test('keeps the platform expiry beyond a slow liveness interval', () {
    final content = buildAgentLiveUpdateContent(
      [
        _turn(
          agent: 'agent-a',
          channel: 'channel-1',
          livenessTimeout: const Duration(hours: 24, seconds: 30),
        ),
      ],
      {'channel-1': 'agents'},
    );

    expect(content!.expiresAt, DateTime.utc(2026, 7, 28, 12, 0, 40));
  });

  test('summarizes additional agents and channels', () {
    final content = buildAgentLiveUpdateContent(
      [
        _turn(agent: 'agent-a', channel: 'channel-1'),
        _turn(agent: 'agent-b', channel: 'channel-2'),
        _turn(agent: 'agent-c', channel: 'channel-3'),
      ],
      {
        'channel-1': 'agents',
        'channel-2': 'design',
        'channel-3': 'engineering',
      },
      {'agent-a': 'Codex', 'agent-b': 'Maya', 'agent-c': 'Theo'},
    );

    expect(content, isNotNull);
    expect(content!.title, 'Codex and 2 more agents are working');
    expect(content.body, 'Across #agents, #design and 1 more');
  });

  test('dismisses the Android update when no agent is active', () async {
    final previousPlatform = debugDefaultTargetPlatformOverride;
    debugDefaultTargetPlatformOverride = TargetPlatform.android;
    final calls = <MethodCall>[];
    const channel = MethodChannel('buzz/agent_live_updates');
    final messenger =
        TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger;
    messenger.setMockMethodCallHandler(channel, (call) async {
      calls.add(call);
      return true;
    });
    addTearDown(() {
      messenger.setMockMethodCallHandler(channel, null);
      debugDefaultTargetPlatformOverride = previousPlatform;
    });

    await syncAgentLiveUpdate(null);

    expect(calls, hasLength(1));
    expect(calls.single.method, 'dismiss');
  });

  test('dismisses the iOS Live Activity when no agent is active', () async {
    final previousPlatform = debugDefaultTargetPlatformOverride;
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    final calls = <MethodCall>[];
    const channel = MethodChannel('buzz/agent_live_updates');
    final messenger =
        TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger;
    messenger.setMockMethodCallHandler(channel, (call) async {
      calls.add(call);
      return true;
    });
    addTearDown(() {
      messenger.setMockMethodCallHandler(channel, null);
      debugDefaultTargetPlatformOverride = previousPlatform;
    });

    await syncAgentLiveUpdate(null);

    expect(calls, hasLength(1));
    expect(calls.single.method, 'dismiss');
  });

  test('serializes rapid show and dismiss platform calls', () async {
    final previousPlatform = debugDefaultTargetPlatformOverride;
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    final calls = <String>[];
    final showGate = Completer<void>();
    const channel = MethodChannel('buzz/agent_live_updates');
    final messenger =
        TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger;
    messenger.setMockMethodCallHandler(channel, (call) async {
      calls.add(call.method);
      if (call.method == 'show') {
        await showGate.future;
      }
      return true;
    });
    addTearDown(() {
      messenger.setMockMethodCallHandler(channel, null);
      debugDefaultTargetPlatformOverride = previousPlatform;
    });

    final content = buildAgentLiveUpdateContent(
      [_turn(agent: 'agent-a', channel: 'channel-1')],
      {'channel-1': 'agents'},
    );
    final synchronizer = AgentLiveUpdateSynchronizer();
    final show = synchronizer.sync(content);
    await Future<void>.delayed(Duration.zero);
    final dismiss = synchronizer.sync(null);
    await Future<void>.delayed(Duration.zero);

    expect(calls, ['show']);
    showGate.complete();
    await Future.wait([show, dismiss]);
    expect(calls, ['show', 'dismiss']);
  });
}

ActiveAgentTurn _turn({
  required String agent,
  required String channel,
  int minute = 0,
  String? message,
  Duration livenessTimeout = const Duration(seconds: 30),
}) {
  final startedAt = DateTime.utc(2026, 7, 27, 12, minute);
  return ActiveAgentTurn(
    agentPubkey: agent,
    channelId: channel,
    turnId: '$agent-$minute',
    startedAt: startedAt,
    lastActivityAt: startedAt.add(const Duration(seconds: 10)),
    livenessTimeout: livenessTimeout,
    triggeringEventId: message,
  );
}
