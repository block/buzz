import 'package:flutter_test/flutter_test.dart';
import 'package:buzz/shared/relay/relay_provider.dart';

void main() {
  group('RelayConfig.wsUrl', () {
    test('converts https to wss', () {
      const config = RelayConfig(baseUrl: 'https://relay.example');
      expect(config.wsUrl, 'wss://relay.example');
    });

    test('converts http to ws', () {
      const config = RelayConfig(baseUrl: 'http://relay.example');
      expect(config.wsUrl, 'ws://relay.example');
    });

    test('preserves wss unchanged', () {
      const config = RelayConfig(baseUrl: 'wss://relay.example');
      expect(config.wsUrl, 'wss://relay.example');
    });

    test('preserves ws unchanged', () {
      const config = RelayConfig(baseUrl: 'ws://relay.example');
      expect(config.wsUrl, 'ws://relay.example');
    });

    test('preserves port with https', () {
      const config = RelayConfig(baseUrl: 'https://relay.example:8443');
      expect(config.wsUrl, 'wss://relay.example:8443');
    });

    test('preserves port with wss', () {
      const config = RelayConfig(baseUrl: 'wss://relay.example:8443');
      expect(config.wsUrl, 'wss://relay.example:8443');
    });

    test('preserves path with http', () {
      const config = RelayConfig(baseUrl: 'http://relay.example:3000/base');
      expect(config.wsUrl, 'ws://relay.example:3000/base');
    });
  });

  group('RelayConfig.httpUrl', () {
    test('preserves https unchanged', () {
      const config = RelayConfig(baseUrl: 'https://relay.example');
      expect(config.httpUrl, 'https://relay.example');
    });

    test('preserves http unchanged', () {
      const config = RelayConfig(baseUrl: 'http://relay.example');
      expect(config.httpUrl, 'http://relay.example');
    });

    test('converts wss to https', () {
      const config = RelayConfig(baseUrl: 'wss://relay.example');
      expect(config.httpUrl, 'https://relay.example');
    });

    test('converts ws to http', () {
      const config = RelayConfig(baseUrl: 'ws://relay.example');
      expect(config.httpUrl, 'http://relay.example');
    });

    test('preserves port with wss', () {
      const config = RelayConfig(baseUrl: 'wss://relay.example:8443');
      expect(config.httpUrl, 'https://relay.example:8443');
    });

    test('preserves path with ws', () {
      const config = RelayConfig(baseUrl: 'ws://relay.example:3000/base');
      expect(config.httpUrl, 'http://relay.example:3000/base');
    });
  });
}
