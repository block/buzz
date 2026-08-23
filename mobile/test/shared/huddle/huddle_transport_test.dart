import 'dart:async';
import 'dart:convert';
import 'dart:typed_data';

import 'package:buzz/shared/huddle/huddle_auth.dart';
import 'package:buzz/shared/huddle/huddle_transport.dart';
import 'package:buzz/shared/huddle/huddle_wire.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:web_socket_channel/web_socket_channel.dart';

const _privateKey =
    '09b3065e3570a3a4054660dccd66e12774a99a904fdb0ca02dbc6c3136249506';
const _parentChannelId = '11111111-2222-4333-8444-555555555555';
const _ephemeralChannelId = 'aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee';

void main() {
  test('authenticates with object JSON and admits a v2 room', () async {
    final channel = _ControlledWebSocketChannel();
    final transport = _transport(channel);
    addTearDown(transport.dispose);

    final connect = transport.connect();
    await _waitForPhase(transport, HuddleTransportPhase.awaitingChallenge);
    channel.emitText(jsonEncode({'type': 'challenge', 'challenge': 'abc'}));
    await _waitForPhase(transport, HuddleTransportPhase.authenticating);

    final auth = jsonDecode(channel.sink.sent.single as String);
    expect(auth, isA<Map<String, dynamic>>());
    expect(auth['type'], 'auth');
    expect(auth['parent_channel_id'], _parentChannelId);
    expect(auth['protocol_version'], 2);

    channel.emitText(
      jsonEncode({
        'type': 'joined',
        'pubkey': 'self',
        'peer_index': 3,
        'peers': [
          {'pubkey': 'desktop', 'peer_index': 1},
        ],
      }),
    );
    await connect;

    expect(transport.state.phase, HuddleTransportPhase.connected);
    expect(transport.state.localPeerIndex, 3);
    expect(transport.state.peers.keys, containsAll([1, 3]));
  });

  test(
    'parses inbound relay media and encodes outbound client media',
    () async {
      final channel = _ControlledWebSocketChannel();
      final transport = _transport(channel);
      addTearDown(transport.dispose);
      await _connect(
        channel,
        transport,
        peers: const [
          {'pubkey': 'remote', 'peer_index': 4},
        ],
      );

      final inbound = expectLater(
        transport.remoteAudioFrames,
        emits(
          isA<HuddleRemoteAudioFrame>()
              .having((frame) => frame.peerIndex, 'peer index', 4)
              .having((frame) => frame.epoch, 'epoch', 0)
              .having((frame) => frame.header.sequence, 'sequence', 9)
              .having((frame) => frame.opusPayload, 'Opus', [0xaa]),
        ),
      );
      channel.emitBinary(
        Uint8List.fromList([4, 0, 9, 0, 0, 3, 0xc0, 0xd8, 0, 0xaa]),
      );
      await inbound;

      transport.sendOpusFrame(
        header: const HuddleAudioHeader(
          sequence: 10,
          timestamp48k: 1920,
          levelDbov: -40,
          flags: 0,
        ),
        opusPayload: Uint8List.fromList([0xbb]),
      );
      expect(
        channel.sink.sent.last,
        Uint8List.fromList([0, 10, 0, 0, 7, 0x80, 0xd8, 0, 0xbb]),
      );
    },
  );

  test('surfaces malformed media as recoverable issue', () async {
    final channel = _ControlledWebSocketChannel();
    final transport = _transport(channel);
    addTearDown(transport.dispose);
    await _connect(channel, transport);

    final issue = expectLater(
      transport.issues,
      emits(
        isA<HuddleTransportError>().having(
          (error) => error.code,
          'code',
          HuddleTransportErrorCode.protocolViolation,
        ),
      ),
    );
    channel.emitBinary(Uint8List.fromList([1, 2, 3]));
    await issue;

    expect(transport.state.phase, HuddleTransportPhase.connected);
  });

  test(
    'revisioned roster is authoritative and deltas are sequential',
    () async {
      final channel = _ControlledWebSocketChannel();
      final transport = _transport(channel);
      addTearDown(transport.dispose);
      await _connect(
        channel,
        transport,
        revision: 4,
        peers: const [
          {'pubkey': 'desktop', 'peer_index': 1},
        ],
      );
      expect(transport.state.peers.keys, [1]);
      expect(transport.state.rosterRevision, 4);

      channel.emitText(
        jsonEncode({
          'type': 'joined',
          'revision': 5,
          'pubkey': 'agent',
          'peer_index': 2,
          'peers': [
            {'pubkey': 'agent', 'peer_index': 2},
          ],
        }),
      );
      await _waitForRevision(transport, 5);
      expect(transport.state.peers.keys, containsAll([1, 2]));

      channel.emitText(
        jsonEncode({
          'type': 'roster',
          'revision': 6,
          'peers': [
            {'pubkey': 'replacement', 'peer_index': 2},
          ],
        }),
      );
      await _waitForRevision(transport, 6);
      expect(transport.state.peers.keys, [2]);
      expect(transport.state.peers[2]?.pubkey, 'replacement');

      channel.emitText(
        jsonEncode({'type': 'roster', 'revision': 5, 'peers': const []}),
      );
      await Future<void>.delayed(Duration.zero);
      expect(transport.state.rosterRevision, 6);
      expect(transport.state.peers[2]?.pubkey, 'replacement');
    },
  );

  test(
    'newer roster arriving before admission remains authoritative',
    () async {
      final channel = _ControlledWebSocketChannel();
      final transport = _transport(channel);
      addTearDown(transport.dispose);

      final connect = transport.connect();
      await _waitForPhase(transport, HuddleTransportPhase.awaitingChallenge);
      channel.emitText(jsonEncode({'type': 'challenge', 'challenge': 'abc'}));
      await _waitForPhase(transport, HuddleTransportPhase.authenticating);
      channel.emitText(
        jsonEncode({
          'type': 'roster',
          'revision': 6,
          'peers': [
            {'pubkey': 'new', 'peer_index': 4},
          ],
        }),
      );
      channel.emitText(
        jsonEncode({
          'type': 'joined',
          'revision': 5,
          'pubkey': 'self',
          'peer_index': 3,
          'peers': [
            {'pubkey': 'old', 'peer_index': 1},
          ],
        }),
      );
      await connect;

      expect(transport.state.localPeerIndex, 3);
      expect(transport.state.rosterRevision, 6);
      expect(transport.state.peers.keys, [4]);
      expect(transport.state.peers[4]?.pubkey, 'new');
    },
  );

  test('revision gap reconnects without applying the gap', () async {
    final channel = _ControlledWebSocketChannel();
    final transport = _transport(channel);
    addTearDown(transport.dispose);
    await _connect(
      channel,
      transport,
      revision: 2,
      peers: const [
        {'pubkey': 'desktop', 'peer_index': 1},
      ],
    );

    channel.emitText(
      jsonEncode({
        'type': 'left',
        'revision': 4,
        'pubkey': 'desktop',
        'peer_index': 1,
      }),
    );
    await _waitForPhase(transport, HuddleTransportPhase.failed);

    expect(transport.state.rosterRevision, 2);
    expect(transport.state.peers[1]?.pubkey, 'desktop');
    expect(transport.state.error?.message, contains('revision gap'));
  });

  test('revisioned admission does not inject an absent local user', () async {
    final channel = _ControlledWebSocketChannel();
    final transport = _transport(channel);
    addTearDown(transport.dispose);
    await _connect(
      channel,
      transport,
      revision: 8,
      peers: const [
        {'pubkey': 'desktop', 'peer_index': 1},
      ],
    );

    expect(transport.state.localPeerIndex, 3);
    expect(transport.state.peers.keys, [1]);
  });

  test('revisioned joined delta reports index reuse', () async {
    final channel = _ControlledWebSocketChannel();
    final transport = _transport(channel);
    addTearDown(transport.dispose);
    await _connect(
      channel,
      transport,
      revision: 1,
      peers: const [
        {'pubkey': 'old', 'peer_index': 4},
      ],
    );
    final events = <HuddlePeerEvent>[];
    final subscription = transport.peerEvents.listen(events.add);
    addTearDown(subscription.cancel);

    channel.emitText(
      jsonEncode({
        'type': 'joined',
        'revision': 2,
        'pubkey': 'new',
        'peer_index': 4,
        'peers': [
          {'pubkey': 'new', 'peer_index': 4},
        ],
      }),
    );
    await _waitForRevision(transport, 2);

    expect(events.single.type, HuddlePeerEventType.replaced);
    expect(events.single.peer.pubkey, 'old');
    expect(events.single.replacement?.pubkey, 'new');
    expect(transport.state.peers[4]?.pubkey, 'new');
  });

  test('revisioned joined delta reports same-pubkey epoch reuse', () async {
    final channel = _ControlledWebSocketChannel();
    final transport = _transport(channel);
    addTearDown(transport.dispose);
    await _connect(
      channel,
      transport,
      revision: 1,
      peers: const [
        {'pubkey': 'same', 'peer_index': 4, 'epoch': 2},
      ],
    );
    final events = <HuddlePeerEvent>[];
    final subscription = transport.peerEvents.listen(events.add);
    addTearDown(subscription.cancel);

    channel.emitText(
      jsonEncode({
        'type': 'joined',
        'revision': 2,
        'pubkey': 'same',
        'peer_index': 4,
        'epoch': 3,
        'peers': [
          {'pubkey': 'same', 'peer_index': 4, 'epoch': 3},
        ],
      }),
    );
    await _waitForRevision(transport, 2);

    expect(events.single.type, HuddlePeerEventType.replaced);
    expect(events.single.peer.epoch, 2);
    expect(events.single.replacement?.epoch, 3);
  });

  test('index replacement is emitted before new media is admitted', () async {
    final channel = _ControlledWebSocketChannel();
    final transport = _transport(channel);
    addTearDown(transport.dispose);
    await _connect(
      channel,
      transport,
      revision: 1,
      peers: const [
        {'pubkey': 'old', 'peer_index': 4},
      ],
    );
    final events = <HuddlePeerEvent>[];
    final subscription = transport.peerEvents.listen(events.add);
    addTearDown(subscription.cancel);

    channel.emitText(
      jsonEncode({
        'type': 'roster',
        'revision': 2,
        'peers': [
          {'pubkey': 'new', 'peer_index': 4},
        ],
      }),
    );
    await _waitForRevision(transport, 2);

    expect(events.single.type, HuddlePeerEventType.replaced);
    expect(events.single.peer.pubkey, 'old');
    expect(events.single.replacement?.pubkey, 'new');
  });

  test('exposes relay rejection code and failed lifecycle state', () async {
    final channel = _ControlledWebSocketChannel();
    final transport = _transport(channel);
    addTearDown(transport.dispose);

    final connect = transport.connect();
    await _waitForPhase(transport, HuddleTransportPhase.awaitingChallenge);
    channel.emitText(jsonEncode({'type': 'challenge', 'challenge': 'abc'}));
    await _waitForPhase(transport, HuddleTransportPhase.authenticating);
    channel.emitText(
      jsonEncode({
        'type': 'error',
        'code': 'upgrade_required',
        'message': 'room is pinned to v1',
      }),
    );

    await expectLater(
      connect,
      throwsA(
        isA<HuddleTransportError>()
            .having(
              (error) => error.code,
              'code',
              HuddleTransportErrorCode.relayRejected,
            )
            .having(
              (error) => error.relayCode,
              'relay code',
              'upgrade_required',
            ),
      ),
    );
    expect(transport.state.phase, HuddleTransportPhase.failed);
  });

  test('bounds transport audio ingress', () async {
    final channel = _ControlledWebSocketChannel();
    final transport = _transport(channel);
    addTearDown(transport.dispose);
    await _connect(
      channel,
      transport,
      revision: 1,
      peers: const [
        {'pubkey': 'remote', 'peer_index': 4},
      ],
    );
    final received = <int>[];
    final drained = Completer<void>();
    final subscription = transport.remoteAudioFrames.listen((frame) {
      received.add(frame.header.sequence);
      if (frame.header.sequence == 99 && !drained.isCompleted) {
        drained.complete();
      }
    });
    addTearDown(subscription.cancel);

    for (var sequence = 0; sequence < 100; sequence++) {
      channel.emitBinary(_relayFrame(peerIndex: 4, sequence: sequence));
    }
    await drained.future.timeout(const Duration(seconds: 1));

    expect(received, hasLength(50));
    expect(received.first, 50);
    expect(received.last, 99);
  });

  test(
    'authoritative left purges queued media before same-index rejoin',
    () async {
      final channel = _ControlledWebSocketChannel();
      final transport = _transport(channel);
      addTearDown(transport.dispose);
      await _connect(
        channel,
        transport,
        revision: 1,
        peers: const [
          {'pubkey': 'old', 'peer_index': 4},
        ],
      );
      final received = <int>[];
      final subscription = transport.remoteAudioFrames.listen(
        (frame) => received.add(frame.header.sequence),
      );
      addTearDown(subscription.cancel);

      for (var sequence = 0; sequence < 50; sequence++) {
        channel.emitBinary(_relayFrame(peerIndex: 4, sequence: sequence));
      }
      channel.emitText(
        jsonEncode({
          'type': 'left',
          'revision': 2,
          'pubkey': 'old',
          'peer_index': 4,
        }),
      );
      channel.emitText(
        jsonEncode({
          'type': 'joined',
          'revision': 3,
          'pubkey': 'new',
          'peer_index': 4,
        }),
      );
      channel.emitBinary(_relayFrame(peerIndex: 4, sequence: 50));
      await _waitForRevision(transport, 3);
      await Future<void>.delayed(const Duration(milliseconds: 20));

      expect(transport.state.peers[4]?.pubkey, 'new');
      expect(received, [50]);
    },
  );

  test(
    'v2 media routes through the current peer-index occupant after reuse',
    () async {
      // Protocol v2 exposes only the peer index on media. After the roster
      // reassigns that index, subsequent frames route to the current occupant.
      final channel = _ControlledWebSocketChannel();
      final transport = _transport(channel);
      addTearDown(transport.dispose);
      await _connect(
        channel,
        transport,
        revision: 1,
        peers: const [
          {'pubkey': 'old', 'peer_index': 4, 'epoch': 0},
        ],
      );
      final received = <int>[];
      final subscription = transport.remoteAudioFrames.listen(
        (frame) => received.add(frame.header.sequence),
      );
      addTearDown(subscription.cancel);

      // 'old' (epoch 0) leaves and 'new' reuses index 4 at epoch 1.
      channel.emitText(
        jsonEncode({
          'type': 'joined',
          'revision': 2,
          'pubkey': 'new',
          'peer_index': 4,
          'epoch': 1,
          'peers': [
            {'pubkey': 'new', 'peer_index': 4, 'epoch': 1},
          ],
        }),
      );
      await _waitForRevision(transport, 2);
      expect(transport.state.peers[4]?.pubkey, 'new');
      expect(transport.state.peers[4]?.epoch, 1);

      // V2 cannot distinguish delayed media from the old occupancy, so both
      // packets are delivered through the currently occupied index.
      channel.emitBinary(_relayFrame(peerIndex: 4, epoch: 0, sequence: 7));
      channel.emitBinary(_relayFrame(peerIndex: 4, epoch: 1, sequence: 8));
      await Future<void>.delayed(const Duration(milliseconds: 20));

      expect(received, [7, 8]);
    },
  );

  test(
    're-admission reports departed and reused peer indexes before new media',
    () async {
      final firstChannel = _ControlledWebSocketChannel();
      final secondChannel = _ControlledWebSocketChannel();
      var connection = 0;
      final transport = HuddleTransport(
        parameters: HuddleConnectionParameters(
          relayWebSocketUrl: 'wss://buzz.example',
          nsec: _privateKey,
          parentChannelId: _parentChannelId,
          ephemeralChannelId: _ephemeralChannelId,
        ),
        channelFactory: (_) => connection++ == 0 ? firstChannel : secondChannel,
        connectTimeout: const Duration(seconds: 1),
        handshakeTimeout: const Duration(seconds: 1),
      );
      addTearDown(transport.dispose);
      await _connect(
        firstChannel,
        transport,
        revision: 1,
        peers: const [
          {'pubkey': 'old', 'peer_index': 4},
          {'pubkey': 'departed', 'peer_index': 5},
        ],
      );
      final events = <HuddlePeerEvent>[];
      final subscription = transport.peerEvents.listen(events.add);
      addTearDown(subscription.cancel);

      firstChannel.emitError(StateError('socket lost'));
      await _waitForPhase(transport, HuddleTransportPhase.failed);
      final reconnect = transport.connect();
      await _waitForPhase(transport, HuddleTransportPhase.awaitingChallenge);
      secondChannel.emitText(
        jsonEncode({'type': 'challenge', 'challenge': 'reconnect'}),
      );
      await _waitForPhase(transport, HuddleTransportPhase.authenticating);
      secondChannel.emitText(
        jsonEncode({
          'type': 'joined',
          'revision': 4,
          'pubkey': 'self',
          'peer_index': 3,
          'peers': [
            {'pubkey': 'new', 'peer_index': 4},
            {'pubkey': 'self', 'peer_index': 3},
          ],
        }),
      );
      await reconnect;

      expect(transport.state.peers[4]?.pubkey, 'new');
      expect(events, hasLength(2));
      expect(events[0].type, HuddlePeerEventType.replaced);
      expect(events[0].peer.pubkey, 'old');
      expect(events[0].replacement?.pubkey, 'new');
      expect(events[1].type, HuddlePeerEventType.left);
      expect(events[1].peer.pubkey, 'departed');
    },
  );

  test('re-admission resets same-pubkey slot when its epoch changes', () async {
    final firstChannel = _ControlledWebSocketChannel();
    final secondChannel = _ControlledWebSocketChannel();
    var connection = 0;
    final transport = HuddleTransport(
      parameters: HuddleConnectionParameters(
        relayWebSocketUrl: 'wss://buzz.example',
        nsec: _privateKey,
        parentChannelId: _parentChannelId,
        ephemeralChannelId: _ephemeralChannelId,
      ),
      channelFactory: (_) => connection++ == 0 ? firstChannel : secondChannel,
      connectTimeout: const Duration(seconds: 1),
      handshakeTimeout: const Duration(seconds: 1),
    );
    addTearDown(transport.dispose);
    await _connect(
      firstChannel,
      transport,
      revision: 1,
      peers: const [
        {'pubkey': 'same', 'peer_index': 4, 'epoch': 0},
      ],
    );
    final events = <HuddlePeerEvent>[];
    final subscription = transport.peerEvents.listen(events.add);
    addTearDown(subscription.cancel);

    firstChannel.emitError(StateError('relay restarted'));
    await _waitForPhase(transport, HuddleTransportPhase.failed);
    final reconnect = transport.connect();
    await _waitForPhase(transport, HuddleTransportPhase.awaitingChallenge);
    secondChannel.emitText(
      jsonEncode({'type': 'challenge', 'challenge': 'reconnect'}),
    );
    await _waitForPhase(transport, HuddleTransportPhase.authenticating);
    secondChannel.emitText(
      jsonEncode({
        'type': 'joined',
        'revision': 1,
        'pubkey': 'self',
        'peer_index': 3,
        'epoch': 0,
        'peers': [
          {'pubkey': 'same', 'peer_index': 4, 'epoch': 1},
          {'pubkey': 'self', 'peer_index': 3, 'epoch': 0},
        ],
      }),
    );
    await reconnect;

    expect(events, hasLength(1));
    expect(events.single.type, HuddlePeerEventType.replaced);
    expect(events.single.peer.epoch, 0);
    expect(events.single.replacement?.epoch, 1);
  });

  test('re-admission resets same slot even when its epoch is reused', () async {
    final firstChannel = _ControlledWebSocketChannel();
    final secondChannel = _ControlledWebSocketChannel();
    var connection = 0;
    final transport = HuddleTransport(
      parameters: HuddleConnectionParameters(
        relayWebSocketUrl: 'wss://buzz.example',
        nsec: _privateKey,
        parentChannelId: _parentChannelId,
        ephemeralChannelId: _ephemeralChannelId,
      ),
      channelFactory: (_) => connection++ == 0 ? firstChannel : secondChannel,
      connectTimeout: const Duration(seconds: 1),
      handshakeTimeout: const Duration(seconds: 1),
    );
    addTearDown(transport.dispose);
    await _connect(
      firstChannel,
      transport,
      revision: 9,
      peers: const [
        {'pubkey': 'same', 'peer_index': 4, 'epoch': 0},
      ],
    );
    final events = <HuddlePeerEvent>[];
    final subscription = transport.peerEvents.listen(events.add);
    addTearDown(subscription.cancel);

    firstChannel.emitError(StateError('relay restarted'));
    await _waitForPhase(transport, HuddleTransportPhase.failed);
    final reconnect = transport.connect();
    await _waitForPhase(transport, HuddleTransportPhase.awaitingChallenge);
    secondChannel.emitText(
      jsonEncode({'type': 'challenge', 'challenge': 'reconnect'}),
    );
    await _waitForPhase(transport, HuddleTransportPhase.authenticating);
    secondChannel.emitText(
      jsonEncode({
        'type': 'joined',
        'revision': 1,
        'pubkey': 'self',
        'peer_index': 3,
        'epoch': 0,
        'peers': [
          {'pubkey': 'same', 'peer_index': 4, 'epoch': 0},
          {'pubkey': 'self', 'peer_index': 3, 'epoch': 0},
        ],
      }),
    );
    await reconnect;

    expect(events, hasLength(1));
    expect(events.single.type, HuddlePeerEventType.replaced);
    expect(events.single.peer.pubkey, 'same');
    expect(events.single.replacement?.pubkey, 'same');
    expect(events.single.replacement?.epoch, 0);
  });

  test('re-admission accepts a lower revision after relay restart', () async {
    final firstChannel = _ControlledWebSocketChannel();
    final secondChannel = _ControlledWebSocketChannel();
    var connection = 0;
    final transport = HuddleTransport(
      parameters: HuddleConnectionParameters(
        relayWebSocketUrl: 'wss://buzz.example',
        nsec: _privateKey,
        parentChannelId: _parentChannelId,
        ephemeralChannelId: _ephemeralChannelId,
      ),
      channelFactory: (_) => connection++ == 0 ? firstChannel : secondChannel,
      connectTimeout: const Duration(seconds: 1),
      handshakeTimeout: const Duration(seconds: 1),
    );
    addTearDown(transport.dispose);
    await _connect(
      firstChannel,
      transport,
      revision: 9,
      peers: const [
        {'pubkey': 'stale', 'peer_index': 4},
      ],
    );

    firstChannel.emitError(StateError('relay restarted'));
    await _waitForPhase(transport, HuddleTransportPhase.failed);
    final reconnect = transport.connect();
    await _waitForPhase(transport, HuddleTransportPhase.awaitingChallenge);
    secondChannel.emitText(
      jsonEncode({'type': 'challenge', 'challenge': 'reconnect'}),
    );
    await _waitForPhase(transport, HuddleTransportPhase.authenticating);
    secondChannel.emitText(
      jsonEncode({
        'type': 'joined',
        'revision': 1,
        'pubkey': 'self',
        'peer_index': 3,
        'peers': [
          {'pubkey': 'fresh', 'peer_index': 4},
          {'pubkey': 'self', 'peer_index': 3},
        ],
      }),
    );
    await reconnect;

    expect(transport.state.rosterRevision, 1);
    expect(transport.state.peers[4]?.pubkey, 'fresh');
    secondChannel.emitText(
      jsonEncode({
        'type': 'joined',
        'revision': 2,
        'pubkey': 'next',
        'peer_index': 5,
      }),
    );
    await _waitForRevision(transport, 2);
    expect(transport.state.peers[5]?.pubkey, 'next');
  });

  test(
    'intentional disconnect reaches disconnected without an error',
    () async {
      final channel = _ControlledWebSocketChannel();
      final transport = _transport(channel);
      addTearDown(transport.dispose);
      await _connect(channel, transport);

      await transport.disconnect();

      expect(transport.state.phase, HuddleTransportPhase.disconnected);
      expect(transport.state.error, isNull);
    },
  );
}

HuddleTransport _transport(_ControlledWebSocketChannel channel) =>
    HuddleTransport(
      parameters: HuddleConnectionParameters(
        relayWebSocketUrl: 'wss://buzz.example',
        nsec: _privateKey,
        parentChannelId: _parentChannelId,
        ephemeralChannelId: _ephemeralChannelId,
      ),
      channelFactory: (_) => channel,
      connectTimeout: const Duration(seconds: 1),
      handshakeTimeout: const Duration(seconds: 1),
    );

Future<void> _connect(
  _ControlledWebSocketChannel channel,
  HuddleTransport transport, {
  int? revision,
  List<Map<String, Object>> peers = const [],
}) async {
  final connect = transport.connect();
  await _waitForPhase(transport, HuddleTransportPhase.awaitingChallenge);
  channel.emitText(jsonEncode({'type': 'challenge', 'challenge': 'abc'}));
  await _waitForPhase(transport, HuddleTransportPhase.authenticating);
  final admission = <String, Object>{
    'type': 'joined',
    'pubkey': 'self',
    'peer_index': 3,
    'peers': peers,
  };
  if (revision != null) admission['revision'] = revision;
  channel.emitText(jsonEncode(admission));
  await connect;
}

Uint8List _relayFrame({
  required int peerIndex,
  required int sequence,
  int epoch = 0,
}) {
  final header = HuddleAudioHeader(
    sequence: sequence,
    timestamp48k: sequence * 960,
    levelDbov: -30,
    flags: 0,
  );
  final clientFrame = HuddleWireV2.encodeClientFrame(
    header,
    Uint8List.fromList([sequence & 0xff]),
  );
  return Uint8List.fromList([peerIndex, ...clientFrame]);
}

Future<void> _waitForPhase(
  HuddleTransport transport,
  HuddleTransportPhase phase,
) async {
  for (var attempt = 0; attempt < 100; attempt++) {
    if (transport.state.phase == phase) return;
    await Future<void>.delayed(const Duration(milliseconds: 1));
  }
  fail('Transport never reached ${phase.name}; was ${transport.state.phase}.');
}

Future<void> _waitForRevision(HuddleTransport transport, int revision) async {
  for (var attempt = 0; attempt < 100; attempt++) {
    if (transport.state.rosterRevision == revision) return;
    await Future<void>.delayed(const Duration(milliseconds: 1));
  }
  fail(
    'Transport never reached roster revision $revision; '
    'was ${transport.state.rosterRevision}.',
  );
}

final class _ControlledWebSocketChannel implements WebSocketChannel {
  final StreamController<dynamic> _streamController = StreamController();
  final _RecordingWebSocketSink _sink = _RecordingWebSocketSink();

  void emitText(String value) => _streamController.add(value);
  void emitBinary(Uint8List value) => _streamController.add(value);
  void emitError(Object error) => _streamController.addError(error);

  @override
  Future<void> get ready => Future.value();

  @override
  Stream<dynamic> get stream => _streamController.stream;

  @override
  _RecordingWebSocketSink get sink => _sink;

  @override
  int? get closeCode => null;

  @override
  String? get closeReason => null;

  @override
  String? get protocol => null;

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

final class _RecordingWebSocketSink implements WebSocketSink {
  final List<dynamic> sent = [];

  @override
  void add(dynamic event) => sent.add(event);

  @override
  void addError(Object error, [StackTrace? stackTrace]) {}

  @override
  Future<void> addStream(Stream<dynamic> stream) async {
    await for (final event in stream) {
      sent.add(event);
    }
  }

  @override
  Future<void> close([int? closeCode, String? closeReason]) async {}

  @override
  Future<void> get done => Future.value();
}
