import 'package:flutter_test/flutter_test.dart';

import 'package:buzz/shared/relay/relay_provider.dart';

void main() {
  group('RelayConfig transport URLs', () {
    for (final testCase in [
      (
        input: 'https://relay.example',
        http: 'https://relay.example',
        ws: 'wss://relay.example',
      ),
      (
        input: 'wss://relay.example',
        http: 'https://relay.example',
        ws: 'wss://relay.example',
      ),
      (
        input: 'http://localhost:3000',
        http: 'http://localhost:3000',
        ws: 'ws://localhost:3000',
      ),
      (
        input: 'ws://localhost:3000',
        http: 'http://localhost:3000',
        ws: 'ws://localhost:3000',
      ),
    ]) {
      test('projects ${testCase.input} onto HTTP and WebSocket', () {
        final config = RelayConfig(baseUrl: testCase.input);

        expect(config.httpUrl, testCase.http);
        expect(config.wsUrl, testCase.ws);
      });
    }

    test('rejects unsupported schemes', () {
      final config = RelayConfig(baseUrl: 'ftp://relay.example');

      expect(() => config.httpUrl, throwsFormatException);
      expect(() => config.wsUrl, throwsFormatException);
    });
  });
}
