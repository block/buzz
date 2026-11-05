import 'dart:async';
import 'dart:convert';
import 'dart:typed_data';

import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channel_detail_page.dart';
import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/features/channels/mobile_huddle_controller.dart';
import 'package:buzz/features/channels/slash_commands/groups/navigation_commands.dart';
import 'package:buzz/shared/huddle/huddle.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/buzz_icons.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:hooks_riverpod/misc.dart' show Override;
import 'package:shared_preferences/shared_preferences.dart';

import '../../../helpers/fake_relay.dart';

const _channelId = '11111111-2222-4333-8444-555555555555';
const _dmId = '99999999-2222-4333-8444-555555555555';
const _bob = 'b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0';
const _aliceHuddleId = 'a11ce000-2222-4333-8444-555555555555';

final _general = Channel(
  id: _channelId,
  name: 'general',
  channelType: 'stream',
  visibility: 'open',
  description: 'General discussion',
  createdBy: 'abc123',
  createdAt: DateTime(2025),
  memberCount: 5,
  isMember: true,
);

class _Resumed extends AppLifecycleNotifier {
  @override
  AppLifecycleState build() => AppLifecycleState.resumed;
}

class _DmActions extends ChannelActions {
  _DmActions(Ref ref, FakeRelay relay, this.dm, this.opened)
    : super(
        ref: ref,
        session: ref.read(relaySessionProvider.notifier),
        signedEventRelay: SignedEventRelay(
          session: ref.read(relaySessionProvider.notifier),
          nsec: relay.keys.nsec,
        ),
        currentPubkey: relay.pubkey,
      );

  final Channel dm;
  final List<List<String>> opened;

  @override
  Future<Channel> openDm({
    required List<String> pubkeys,
    bool waitForChannelList = true,
  }) async {
    opened.add(pubkeys);
    return dm;
  }
}

class _Media implements HuddleMedia {
  final _states = StreamController<HuddleMediaState>.broadcast(sync: true);
  final _frames = StreamController<HuddleLocalAudioFrame>.broadcast(sync: true);
  var _state = const HuddleMediaState(phase: HuddleMediaPhase.idle);

  @override
  HuddleMediaState get state => _state;
  @override
  Stream<HuddleMediaState> get states => _states.stream;
  @override
  Stream<HuddleLocalAudioFrame> get localAudioFrames => _frames.stream;

  @override
  Future<HuddleMediaCapabilities> discoverCapabilities() async {
    const capabilities = HuddleMediaCapabilities(
      platform: 'test',
      supportsAudioSession: true,
      supportsMicrophonePermission: true,
      supportsCapture: true,
      supportsPlayback: true,
      supportsOpusEncoding: true,
      supportsOpusDecoding: true,
    );
    _state = const HuddleMediaState(
      phase: HuddleMediaPhase.idle,
      capabilities: capabilities,
    );
    return capabilities;
  }

  @override
  Future<HuddleMicrophonePermission> requestMicrophonePermission() async =>
      HuddleMicrophonePermission.granted;

  void _emit(HuddleMediaPhase phase) {
    _state = HuddleMediaState(phase: phase, capabilities: _state.capabilities);
    _states.add(_state);
  }

  @override
  Future<void> prepare() async => _emit(HuddleMediaPhase.prepared);
  @override
  Future<void> start() async => _emit(HuddleMediaPhase.active);
  @override
  Future<void> stop() async => _emit(HuddleMediaPhase.stopped);
  @override
  Future<void> dispose() => stop();
  @override
  Future<bool> openSystemSettings() async => true;
  @override
  Future<void> setMuted(bool muted) async {}
  @override
  Future<void> setSpeakerEnabled(bool enabled) async {}
  @override
  Future<void> playRemoteFrame(HuddleRemoteAudioFrame frame) async {}
  @override
  Future<void> removeRemotePeer(int peerIndex) async {}
}

class _Transport implements HuddleTransportClient {
  final _states = StreamController<HuddleTransportState>.broadcast(sync: true);
  final _frames = StreamController<HuddleRemoteAudioFrame>.broadcast(
    sync: true,
  );
  final _peers = StreamController<HuddlePeerEvent>.broadcast(sync: true);
  final _issues = StreamController<HuddleTransportError>.broadcast(sync: true);
  var _state = HuddleTransportState.idle();

  @override
  HuddleTransportState get state => _state;
  @override
  Stream<HuddleTransportState> get states => _states.stream;
  @override
  Stream<HuddleRemoteAudioFrame> get remoteAudioFrames => _frames.stream;
  @override
  Stream<HuddlePeerEvent> get peerEvents => _peers.stream;
  @override
  Stream<HuddleTransportError> get issues => _issues.stream;

  @override
  Future<void> connect() async {
    _state = HuddleTransportState(
      phase: HuddleTransportPhase.connected,
      localPeerIndex: 1,
      peers: const {1: HuddlePeer(pubkey: 'self', peerIndex: 1, epoch: 0)},
    );
    _states.add(_state);
  }

  @override
  void sendOpusFrame({
    required HuddleAudioHeader header,
    required Uint8List opusPayload,
  }) {}
  @override
  Future<void> disconnect() async {}
  @override
  Future<void> dispose() async {}
}

Future<({FakeRelay relay, List<List<String>> opened})> _pump(
  WidgetTester tester,
) async {
  SharedPreferences.setMockInitialValues({});
  final prefs = await SharedPreferences.getInstance();
  tester.view.physicalSize = const Size(390, 844);
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);
  final relay = FakeRelay();
  final opened = <List<String>>[];
  final dm = Channel(
    id: _dmId,
    name: 'Bob',
    channelType: 'dm',
    visibility: 'private',
    description: '',
    participantPubkeys: [relay.pubkey, _bob],
    createdBy: relay.pubkey,
    createdAt: DateTime(2025),
    memberCount: 2,
    isMember: true,
  );
  final List<Override> overrides = [
    ...relay.overrides,
    savedPrefsProvider.overrideWithValue(prefs),
    appLifecycleProvider.overrideWith(_Resumed.new),
    channelActionsProvider.overrideWith(
      (ref) => _DmActions(ref, relay, dm, opened),
    ),
    channelMembersProvider(_channelId).overrideWith(
      (ref) async => [
        ChannelMember(
          pubkey: _bob,
          role: 'member',
          joinedAt: DateTime(2025),
          displayName: 'Bob',
        ),
      ],
    ),
    huddleMediaFactoryProvider.overrideWithValue(_Media.new),
    huddleTransportFactoryProvider.overrideWithValue((_) => _Transport()),
    huddleHumanCountProvider.overrideWithValue((_) async => 1),
  ];
  await tester.pumpWidget(
    ProviderScope(
      key: UniqueKey(),
      overrides: overrides,
      child: MaterialApp(
        theme: AppTheme.light(),
        home: ChannelDetailPage(channel: _general),
      ),
    ),
  );
  await tester.pumpAndSettle();
  return (relay: relay, opened: opened);
}

String _draft(WidgetTester tester) =>
    tester.widget<TextField>(find.byType(TextField)).controller!.text;

Future<void> _type(WidgetTester tester, String text) async {
  if (find.byType(TextField).evaluate().isEmpty) {
    await tester.tap(find.text('Message #general'));
    await tester.pumpAndSettle();
  }
  await tester.enterText(find.byType(TextField), text);
  await tester.pump();
}

Future<void> _send(WidgetTester tester, {bool settle = true}) async {
  await tester.tap(find.byIcon(BuzzIcons.arrowUp));
  if (settle) {
    await tester.pumpAndSettle();
    return;
  }
  await _pumpFor(tester);
}

Future<void> _pumpFor(WidgetTester tester) async {
  for (var i = 0; i < 10; i++) {
    await tester.pump(const Duration(milliseconds: 100));
  }
}

Future<void> _dismissSnackBar(WidgetTester tester) async {
  await tester.pump(const Duration(seconds: 6));
  await tester.pumpAndSettle();
}

Iterable<NostrEvent> _messagesIn(FakeRelay relay, String channelId) =>
    relay.stored.where(
      (e) =>
          e.kind == EventKind.streamMessage && e.getTagValue('h') == channelId,
    );

Iterable<String> _pageChannelIds(WidgetTester tester) => [
  for (final page in tester.widgetList<ChannelDetailPage>(
    find.byType(ChannelDetailPage, skipOffstage: false),
  ))
    page.channel.id,
];

void main() {
  group('parseDmArgs', () {
    const bob = (pubkey: 'bob', label: 'Bob');
    const maryJane = (pubkey: 'mj', label: 'Mary Jane');
    final cases = <String, ({List<String> to, String message})>{
      '@Bob': (to: ['bob'], message: ''),
      '@Bob hey there': (to: ['bob'], message: 'hey there'),
      '@Bob @Mary Jane  lunch?': (to: ['bob', 'mj'], message: 'lunch?'),
      '@Bob ping @Mary Jane': (to: ['bob'], message: 'ping @Mary Jane'),
      'hey @Bob': (to: <String>[], message: 'hey @Bob'),
      '': (to: <String>[], message: ''),
    };
    for (final MapEntry(:key, :value) in cases.entries) {
      test('"$key"', () {
        final parsed = parseDmArgs(key, [bob, maryJane]);
        expect([for (final r in parsed.recipients) r.pubkey], value.to);
        expect(parsed.message, value.message);
      });
    }
  });

  testWidgets('/dm and its /msg alias open the DM, send there first, and stay '
      'put on a failed send', (tester) async {
    final (:relay, :opened) = await _pump(tester);

    await _type(tester, '/dm @bob');
    await _type(tester, '/dm @bob ');
    expect(_draft(tester), '/dm @Bob ');
    await _type(tester, '/dm @Bob hey');

    relay.rejectNext(
      'blocked: not allowed here',
      match: (e) =>
          e.kind == EventKind.streamMessage && e.getTagValue('h') == _dmId,
    );
    await _send(tester);
    expect(opened, [
      [_bob],
    ]);
    expect(_draft(tester), '/dm @Bob hey', reason: 'a failed send keeps it');
    expect(find.byType(SnackBar), findsOneWidget);
    expect(_pageChannelIds(tester), [_channelId], reason: 'no navigation');
    expect(_messagesIn(relay, _channelId), isEmpty);

    await _dismissSnackBar(tester);
    await _send(tester);
    final dmMessage = _messagesIn(relay, _dmId).last;
    expect(dmMessage.content, 'hey');
    expect(
      [
        for (final tag in dmMessage.tags)
          if (tag.first == 'p') tag[1],
      ],
      [_bob],
    );
    expect(
      relay.published.where(
        (e) => e.kind == EventKind.streamMessage && e.content == 'hey',
      ),
      hasLength(2),
      reason: 'one rejected attempt and one accepted; never two accepted',
    );
    expect(_messagesIn(relay, _channelId), isEmpty);
    expect(_pageChannelIds(tester), [_channelId, _dmId]);

    Navigator.of(tester.element(find.byType(ChannelDetailPage).last)).pop();
    await tester.pumpAndSettle();

    await _type(tester, '/msg @bob ');
    await _send(tester);
    expect(opened, hasLength(3));
    expect(_messagesIn(relay, _dmId), hasLength(1), reason: 'no message');
    expect(_pageChannelIds(tester), [_channelId, _dmId]);
    Navigator.of(tester.element(find.byType(ChannelDetailPage).last)).pop();
    await tester.pumpAndSettle();

    await _type(tester, '/dm hey');
    await _send(tester);
    expect(opened, hasLength(3), reason: 'no mention opens nothing');
    expect(find.textContaining('Usage: /dm @user'), findsOneWidget);
    expect(_draft(tester), '/dm hey');
  });

  testWidgets('/huddle starts a huddle, then joins a running one', (
    tester,
  ) async {
    final (:relay, :opened) = await _pump(tester);
    final container = ProviderScope.containerOf(
      tester.element(find.byType(ChannelDetailPage)),
    );
    HuddleSessionState session() => container.read(huddleSessionProvider);
    Iterable<NostrEvent> started() =>
        relay.published.where((e) => e.kind == EventKind.huddleStarted);

    relay.rejectNext('blocked: not allowed here', match: (e) => e.kind == 9007);
    await _type(tester, '/huddle');
    await _send(tester);
    expect(find.byType(SnackBar), findsOneWidget);
    expect(_draft(tester), '/huddle', reason: 'a failed command keeps it');
    expect(session().isInSession, isFalse);
    expect(find.byKey(const ValueKey('huddle-minimize')), findsNothing);

    await _dismissSnackBar(tester);
    await _send(tester, settle: false);
    expect(session().error, isNull);
    expect(session().phase, HuddleSessionPhase.connected);
    expect(session().parentChannelId, _channelId);
    expect(session().isCreator, isTrue);
    expect(started(), hasLength(1));
    expect(find.byKey(const ValueKey('huddle-minimize')), findsOneWidget);

    await tester.tap(find.byTooltip('Leave Huddle'));
    await _pumpFor(tester);
    expect(session().isInSession, isFalse);

    relay.deliver(
      relay.sign(
        kind: EventKind.huddleStarted,
        content: jsonEncode({'ephemeral_channel_id': _aliceHuddleId}),
        createdAt: DateTime.now().millisecondsSinceEpoch ~/ 1000 + 5,
        tags: [
          ['h', _channelId],
        ],
      ),
    );
    await _pumpFor(tester);
    await _type(tester, '/huddle');
    await _send(tester, settle: false);
    expect(session().ephemeralChannelId, _aliceHuddleId);
    expect(session().isCreator, isFalse);
    expect(started(), hasLength(1), reason: 'joining starts nothing new');
    expect(find.byKey(const ValueKey('huddle-minimize')), findsOneWidget);
  });
}
