import 'package:buzz/features/huddles/huddle_controller.dart';
import 'package:buzz/features/huddles/huddle_controls.dart';
import 'package:buzz/features/huddles/huddle_transport.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

void main() {
  test('active Huddle lifecycle selects live room and honors end', () {
    final events = [
      _event(EventKind.huddleStarted, 1, 'room-a'),
      _event(EventKind.huddleEnded, 2, 'room-a'),
      _event(EventKind.huddleStarted, 3, 'room-b'),
    ];

    expect(activeHuddleChannelId(events), 'room-b');
    expect(
      activeHuddleChannelId([
        ...events,
        _event(EventKind.huddleEnded, 4, 'room-b'),
      ]),
      isNull,
    );
    expect(
      activeHuddleChannelId([
        _event(EventKind.huddleStarted, 1, 'room-a'),
        _event(EventKind.huddleStarted, 2, 'room-b'),
        _event(EventKind.huddleEnded, 3, 'room-b'),
      ]),
      'room-a',
    );
  });

  testWidgets(
    'controls expose connected, mute, route, roster, and leave actions',
    (tester) async {
      await tester.pumpWidget(
        ProviderScope(
          overrides: [
            huddleControllerProvider.overrideWith(_ConnectedController.new),
          ],
          child: const MaterialApp(home: Scaffold(body: HuddleControlsSheet())),
        ),
      );

      expect(find.text('Connected'), findsOneWidget);
      expect(find.text('2 participants'), findsOneWidget);
      expect(find.byTooltip('Mute'), findsOneWidget);
      expect(find.byTooltip('Audio output'), findsOneWidget);
      expect(find.byTooltip('Talk to bots'), findsOneWidget);
      expect(find.byTooltip('Leave Huddle'), findsOneWidget);
    },
  );

  testWidgets('controls show capability errors and disable active actions', (
    tester,
  ) async {
    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          huddleControllerProvider.overrideWith(_FailedController.new),
        ],
        child: const MaterialApp(home: Scaffold(body: HuddleControlsSheet())),
      ),
    );
    expect(find.text('Could not connect'), findsOneWidget);
    expect(find.text('Microphone permission is required'), findsOneWidget);
    expect(
      tester.widget<IconButton>(find.byType(IconButton).first).onPressed,
      isNull,
    );
  });
}

NostrEvent _event(int kind, int createdAt, String channelId) => NostrEvent(
  id: '$kind-$createdAt',
  pubkey: 'pubkey',
  createdAt: createdAt,
  kind: kind,
  tags: const [],
  content: '{"ephemeral_channel_id":"$channelId"}',
  sig: 'sig',
);

class _ConnectedController extends HuddleController {
  @override
  HuddleState build() => const HuddleState(
    phase: HuddlePhase.connected,
    parentChannelId: 'parent',
    channelId: 'huddle',
    botPubkeys: {'bot'},
    peers: {
      1: HuddlePeer(pubkey: 'a', peerIndex: 1),
      2: HuddlePeer(pubkey: 'b', peerIndex: 2),
    },
  );
}

class _FailedController extends HuddleController {
  @override
  HuddleState build() => const HuddleState(
    phase: HuddlePhase.failed,
    error: 'Microphone permission is required',
  );
}
