import 'package:flutter_test/flutter_test.dart';

import 'package:buzz/shared/relay/relay_provider.dart';

void main() {
  group('RelayConfig.wsUrl', () {
    test('maps http-family schemes to their websocket equivalent', () {
      expect(
        const RelayConfig(baseUrl: 'https://relay.example').wsUrl,
        'wss://relay.example',
      );
      expect(
        const RelayConfig(baseUrl: 'http://localhost:3000').wsUrl,
        'ws://localhost:3000',
      );
    });

    test('passes ws-family schemes through without downgrading', () {
      // An invite-joined community stores relayUrl as wss://…; downgrading it to
      // plaintext ws:// is a TLS downgrade and fails against TLS-only relays.
      expect(
        const RelayConfig(baseUrl: 'wss://relay.example').wsUrl,
        'wss://relay.example',
      );
      expect(
        const RelayConfig(baseUrl: 'ws://localhost:3000').wsUrl,
        'ws://localhost:3000',
      );
    });
  });
}
