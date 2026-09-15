import 'dart:async';
import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/misc.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';
import 'package:nostr/nostr.dart' as nostr;

import 'package:buzz/features/channels/agent_activity/agent_activity_sheet.dart';
import 'package:buzz/shared/crypto/nip44.dart';
import 'package:buzz/shared/profile/user_cache_provider.dart';
import 'package:buzz/shared/profile/user_profile.dart';
import 'package:buzz/shared/relay/relay.dart';

import '../../../helpers/widget_helpers.dart';

void main() {
  testWidgets('01 error state shows retry affordance', (tester) async {
    final ownerKeychain = nostr.Keys.generate();
    final agentKeychain = nostr.Keys.generate();
    final session = _ScriptedRelaySession();

    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: _overrides(
          session: session,
          nsec: ownerKeychain.nsec,
          agentPubkey: agentKeychain.public,
        ),
        child: AgentActivitySheet(
          channelId: 'test-channel',
          agentPubkey: agentKeychain.public,
        ),
      ),
    );
    // Let the initial subscription microtask run and settle to open.
    await tester.pump();
    await tester.pump();

    // The relay closes the observer subscription while the session stays
    // connected — the terminal dead end this item fixes.
    session.closeAll('boom');
    await tester.pump();

    expect(find.byIcon(LucideIcons.circleX), findsOneWidget);
    expect(find.text('Try again'), findsOneWidget);
    expect(find.byIcon(LucideIcons.rotateCcw), findsOneWidget);
  });

  testWidgets('02 tapping retry re-subscribes and streams live frames again', (
    tester,
  ) async {
    final ownerKeychain = nostr.Keys.generate();
    final agentKeychain = nostr.Keys.generate();
    final session = _ScriptedRelaySession();

    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: _overrides(
          session: session,
          nsec: ownerKeychain.nsec,
          agentPubkey: agentKeychain.public,
        ),
        child: AgentActivitySheet(
          channelId: 'test-channel',
          agentPubkey: agentKeychain.public,
        ),
      ),
    );
    await tester.pump();
    await tester.pump();

    // Terminal error via onClosed.
    session.closeAll('boom');
    await tester.pump();
    expect(find.text('Try again'), findsOneWidget);

    // The retry attempt hangs on a gate so the connecting state is observable.
    session.gateNextSubscribe = true;
    await tester.tap(find.text('Try again'));
    await tester.pump();
    expect(find.byIcon(LucideIcons.circleX), findsNothing);

    // Complete the re-subscribe: filter re-sent, connection opens. (This fake
    // keeps closed subscriptions in `filters`, so 2 = initial + retry.)
    session.releaseSubscribe(0);
    await tester.pump();
    expect(session.filters, hasLength(2));
    expect(session.filters.last.kinds, [EventKind.agentObserverFrame]);
    expect(find.text('Try again'), findsNothing);

    // A live frame arrives on the NEW subscription; the transcript resumes.
    session.emit(
      _observerEvent(
        ownerKeychain: ownerKeychain,
        agentKeychain: agentKeychain,
        payload: _observerFrameJson(seq: 1),
      ),
    );
    await tester.pump();

    expect(find.byType(ListView), findsOneWidget);
    expect(find.text('Try again'), findsNothing);
  });

  testWidgets('03 no retry affordance while healthy or connecting', (
    tester,
  ) async {
    final ownerKeychain = nostr.Keys.generate();
    final agentKeychain = nostr.Keys.generate();
    final session = _ScriptedRelaySession();

    // Gate the FIRST subscribe so the sheet sits in connecting, not open.
    session.gateNextSubscribe = true;
    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: _overrides(
          session: session,
          nsec: ownerKeychain.nsec,
          agentPubkey: agentKeychain.public,
        ),
        child: AgentActivitySheet(
          channelId: 'test-channel',
          agentPubkey: agentKeychain.public,
        ),
      ),
    );
    await tester.pump();
    // Still connecting — no affordance.
    expect(find.text('Try again'), findsNothing);
    session.releaseSubscribe(0);
    await tester.pump();
    // Open and healthy — still no affordance, just the waiting empty state.
    expect(find.text('Try again'), findsNothing);
    expect(find.text('Waiting for activity…'), findsOneWidget);
  });
}

List<Override> _overrides({
  required _ScriptedRelaySession session,
  required String nsec,
  required String agentPubkey,
}) {
  return [
    relaySessionProvider.overrideWith(() => session),
    relayConfigProvider.overrideWith(() => _FakeRelayConfigNotifier(nsec)),
    userCacheProvider.overrideWith(
      () => _SeededUserCacheNotifier({
        agentPubkey.toLowerCase(): UserProfile(
          pubkey: agentPubkey,
          displayName: 'Test Agent',
        ),
      }),
    ),
  ];
}

Map<String, dynamic> _observerFrameJson({required int seq}) => {
  'seq': seq,
  'timestamp': '2026-09-03T09:00:0$seq.000Z',
  'kind': 'turn_started',
  'channelId': 'test-channel',
  'turnId': 'turn-1',
  'payload': {
    'triggeringEventIds': ['$seq'],
  },
};

NostrEvent _observerEvent({
  required nostr.Keys ownerKeychain,
  required nostr.Keys agentKeychain,
  required Map<String, dynamic> payload,
}) {
  final conversationKey = getConversationKey(
    agentKeychain.secret,
    ownerKeychain.public,
  );
  final event = nostr.Event.from(
    kind: EventKind.agentObserverFrame,
    content: nip44Encrypt(conversationKey, jsonEncode(payload)),
    tags: [
      ['p', ownerKeychain.public],
      ['agent', agentKeychain.public],
      ['frame', 'telemetry'],
    ],
    secretKey: agentKeychain.secret,
    verify: false,
  );
  return NostrEvent.fromJson(event.toMap());
}

/// Relay session fake mirroring `_RecordingRelaySession` from the notifier's
/// unit test, plus a gate so a subscribe can be held in-flight.
class _ScriptedRelaySession extends RelaySessionNotifier {
  final List<NostrFilter> filters = [];
  final List<void Function(NostrEvent)> _listeners = [];
  final List<void Function(String message)> _closedListeners = [];
  final List<Completer<void>> _subscribeGates = [];
  bool gateNextSubscribe = false;

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  @override
  Future<void Function()> subscribe(
    NostrFilter filter,
    void Function(NostrEvent) onEvent, {
    void Function(String message)? onClosed,
  }) async {
    filters.add(filter);
    if (gateNextSubscribe) {
      gateNextSubscribe = false;
      final gate = Completer<void>();
      _subscribeGates.add(gate);
      await gate.future;
    }
    _listeners.add(onEvent);
    if (onClosed != null) {
      _closedListeners.add(onClosed);
    }
    return () {
      filters.remove(filter);
      _listeners.remove(onEvent);
      if (onClosed != null) {
        _closedListeners.remove(onClosed);
      }
    };
  }

  void emit(NostrEvent event) {
    for (final listener in List.of(_listeners)) {
      listener(event);
    }
  }

  void closeAll(String message) {
    for (final listener in List.of(_closedListeners)) {
      listener(message);
    }
    _listeners.clear();
    _closedListeners.clear();
  }

  void releaseSubscribe(int index) {
    final gate = _subscribeGates[index];
    if (!gate.isCompleted) {
      gate.complete();
    }
  }
}

class _FakeRelayConfigNotifier extends RelayConfigNotifier {
  final String? _nsec;

  _FakeRelayConfigNotifier(this._nsec);

  @override
  RelayConfig build() =>
      RelayConfig(baseUrl: 'http://localhost:3000', nsec: _nsec);
}

/// Skips the relay-backed batch loader; the sheet only needs a label.
class _SeededUserCacheNotifier extends UserCacheNotifier {
  final Map<String, UserProfile> _seed;

  _SeededUserCacheNotifier(this._seed);

  @override
  Map<String, UserProfile> build() => _seed;

  @override
  Future<bool> preload(List<String> pubkeys) async => true;
}
