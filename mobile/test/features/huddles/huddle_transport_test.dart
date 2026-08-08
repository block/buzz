import 'dart:convert';

import 'package:buzz/features/huddles/huddle_transport.dart';
import 'package:buzz/features/huddles/huddle_wire.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:nostr/nostr.dart' as nostr;

void main() {
  test('auth signs the exact relay challenge and negotiates v2', () {
    final keys = nostr.Keys.generate();
    final message =
        jsonDecode(
              buildHuddleAuthMessage(
                relayWebSocket: Uri.parse('wss://relay.example'),
                challenge: 'challenge-123',
                parentChannelId: '00000000-0000-0000-0000-000000000001',
                nsec: keys.nsec,
              ),
            )
            as Map<String, dynamic>;
    expect(message['type'], 'auth');
    expect(message['protocol_version'], huddleProtocolVersion);
    expect(
      message['parent_channel_id'],
      '00000000-0000-0000-0000-000000000001',
    );
    final event = message['event'] as Map<String, dynamic>;
    expect(event['kind'], 22242);
    expect(event['tags'], contains(equals(['relay', 'wss://relay.example'])));
    expect(event['tags'], contains(equals(['challenge', 'challenge-123'])));
  });
}
