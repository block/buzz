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

  group('existing wss:// community HTTP derivation', () {
    // Exercises the scenario where a community was stored with a wss:// URL
    // (invite-created before migration). Even without the migration running,
    // httpUrl should produce a valid HTTPS URL for REST endpoints.
    test('httpUrl converts stored wss:// community URL to https', () {
      // Simulates a RelayConfig built from an un-migrated community.
      const config = RelayConfig(
        baseUrl: 'wss://hosted.communities.buzz.xyz',
        nsec: 'nsec1test',
      );
      expect(config.httpUrl, 'https://hosted.communities.buzz.xyz');
      expect(config.wsUrl, 'wss://hosted.communities.buzz.xyz');
    });

    test('httpUrl converts stored ws:// community URL to http', () {
      const config = RelayConfig(baseUrl: 'ws://localhost:3000');
      expect(config.httpUrl, 'http://localhost:3000');
      expect(config.wsUrl, 'ws://localhost:3000');
    });
  });
}
