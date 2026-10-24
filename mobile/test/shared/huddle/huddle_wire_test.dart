import 'dart:typed_data';

import 'package:buzz/shared/huddle/huddle_auth.dart';
import 'package:buzz/shared/huddle/huddle_wire.dart';
import 'package:flutter_test/flutter_test.dart';

const _privateKey =
    '09b3065e3570a3a4054660dccd66e12774a99a904fdb0ca02dbc6c3136249506';
const _parentChannelId = '11111111-2222-4333-8444-555555555555';
const _ephemeralChannelId = 'aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee';

void main() {
  group('HuddleWireV2', () {
    test('encodes the desktop-compatible big-endian v2 golden frame', () {
      const header = HuddleAudioHeader(
        sequence: 0x1234,
        timestamp48k: 0x01020304,
        levelDbov: -42,
        flags: HuddleWireV2.dtxFlag,
      );

      expect(
        HuddleWireV2.encodeClientFrame(
          header,
          Uint8List.fromList([0xaa, 0xbb]),
        ),
        [0x12, 0x34, 0x01, 0x02, 0x03, 0x04, 0xd6, 0x01, 0xaa, 0xbb],
      );
    });

    test('decodes peer prefix, v2 header, and Opus payload', () {
      final frame = HuddleWireV2.decodeRelayFrame(
        Uint8List.fromList([
          0x07,
          0x12,
          0x34,
          0x01,
          0x02,
          0x03,
          0x04,
          0xd6,
          0x01,
          0xaa,
          0xbb,
        ]),
      );

      expect(frame.peerIndex, 7);
      expect(frame.epoch, 0);
      expect(frame.header.sequence, 0x1234);
      expect(frame.header.timestamp48k, 0x01020304);
      expect(frame.header.levelDbov, -42);
      expect(frame.header.isDtx, isTrue);
      expect(frame.opusPayload, [0xaa, 0xbb]);
    });

    test('coerces invalid positive dBov telemetry to the silent floor', () {
      final header = HuddleAudioHeader.decode(
        Uint8List.fromList([0, 1, 0, 0, 3, 0xc0, 1, 0]),
      );

      expect(header.levelDbov, -127);
    });

    test('checked headers enforce wire ranges outside debug assertions', () {
      expect(
        () => HuddleAudioHeader.checked(
          sequence: -1,
          timestamp48k: 0,
          levelDbov: -127,
          flags: 0,
        ),
        throwsRangeError,
      );
    });

    test('rejects empty and oversized payloads', () {
      const header = HuddleAudioHeader(
        sequence: 0,
        timestamp48k: 0,
        levelDbov: -127,
        flags: 0,
      );

      expect(
        () => HuddleWireV2.encodeClientFrame(header, Uint8List(0)),
        throwsA(isA<HuddleWireException>()),
      );
      expect(
        () => HuddleWireV2.encodeClientFrame(
          header,
          Uint8List(HuddleWireV2.maxBinaryFrameLength),
        ),
        throwsA(isA<HuddleWireException>()),
      );
    });
  });

  group('HuddleAuthV2', () {
    test('uses the base relay URL and fixed v2 auth envelope', () {
      final parameters = HuddleConnectionParameters(
        relayWebSocketUrl: 'wss://buzz.example',
        nsec: _privateKey,
        parentChannelId: _parentChannelId,
        ephemeralChannelId: _ephemeralChannelId,
      );

      final auth = HuddleAuthV2.buildMessage(
        parameters: parameters,
        challenge: 'relay-challenge',
        createdAt: 1_700_000_000,
      );
      final event = auth['event']! as Map<String, dynamic>;
      final tags = (event['tags']! as List<dynamic>)
          .map((tag) => (tag as List<dynamic>).cast<String>())
          .toList();

      expect(auth['type'], 'auth');
      expect(auth['protocol_version'], 2);
      expect(auth['parent_channel_id'], _parentChannelId);
      expect(event['kind'], 22242);
      expect(tags[0], ['relay', 'wss://buzz.example']);
      expect(tags[1], ['challenge', 'relay-challenge']);
      expect(
        parameters.audioWebSocketUri.toString(),
        'wss://buzz.example/huddle/$_ephemeralChannelId/audio',
      );
      expect(
        tags.any((tag) => tag.length > 1 && tag[1].contains('/huddle/')),
        isFalse,
      );
    });

    test('rejects malformed channel IDs before opening a socket', () {
      expect(
        () => HuddleConnectionParameters(
          relayWebSocketUrl: 'wss://buzz.example',
          nsec: _privateKey,
          parentChannelId: 'not-a-channel',
          ephemeralChannelId: _ephemeralChannelId,
        ),
        throwsArgumentError,
      );
    });
  });
}
