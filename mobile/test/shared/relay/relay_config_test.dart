import 'package:flutter_test/flutter_test.dart';

import 'package:buzz/shared/relay/relay_provider.dart';

void main() {
  group('RelayConfig.baseUrl normalization', () {
    test('folds a wss:// community URL to https://', () {
      // Invite joins persist the relay URL straight off the invite link, which
      // deep_link.dart always emits as ws:// or wss://.
      final config = RelayConfig(baseUrl: 'wss://relay.example.com');
      expect(config.baseUrl, 'https://relay.example.com');
    });

    test('folds a ws:// community URL to http://', () {
      final config = RelayConfig(baseUrl: 'ws://relay.example.com:3000');
      expect(config.baseUrl, 'http://relay.example.com:3000');
    });

    test('leaves an https:// community URL untouched', () {
      // Device pairing rejects anything but https://, so these already conform.
      final config = RelayConfig(baseUrl: 'https://relay.example.com');
      expect(config.baseUrl, 'https://relay.example.com');
    });

    test('leaves an http:// community URL untouched', () {
      final config = RelayConfig(baseUrl: 'http://localhost:3000');
      expect(config.baseUrl, 'http://localhost:3000');
    });

    test('preserves a non-default port', () {
      final config = RelayConfig(baseUrl: 'wss://relay.example.com:8443');
      expect(config.baseUrl, 'https://relay.example.com:8443');
    });
  });

  group('RelayConfig.wsUrl', () {
    test('keeps TLS for a relay joined by invite', () {
      // Regression: a wss:// base used to fall through to the non-https branch
      // and downgrade to ws://, dialing port 80 — which never connects on a
      // relay that only serves 443, and drops TLS everywhere else.
      final config = RelayConfig(baseUrl: 'wss://relay.example.com');
      expect(config.wsUrl, 'wss://relay.example.com');
    });

    test('keeps TLS for a relay added by pairing', () {
      final config = RelayConfig(baseUrl: 'https://relay.example.com');
      expect(config.wsUrl, 'wss://relay.example.com');
    });

    test('both onboarding paths agree on the same relay', () {
      final invited = RelayConfig(baseUrl: 'wss://relay.example.com');
      final paired = RelayConfig(baseUrl: 'https://relay.example.com');
      expect(invited.wsUrl, paired.wsUrl);
      expect(invited.baseUrl, paired.baseUrl);
    });

    test('stays plaintext for local development', () {
      final config = RelayConfig(baseUrl: 'http://localhost:3000');
      expect(config.wsUrl, 'ws://localhost:3000');
    });

    test('preserves a non-default port', () {
      final config = RelayConfig(baseUrl: 'wss://relay.example.com:8443');
      expect(config.wsUrl, 'wss://relay.example.com:8443');
    });
  });

  group('Campus / LAN relay', () {
    test('normalizes a bare private address', () {
      expect(normalizeLanRelayUrl('10.24.11.82:3000'), 'ws://10.24.11.82:3000');
    });

    test('accepts private and Tailscale IPv4 addresses', () {
      expect(
        normalizeLanRelayUrl('ws://192.168.1.5:3000'),
        'ws://192.168.1.5:3000',
      );
      expect(
        normalizeLanRelayUrl('ws://100.71.241.45:3000'),
        'ws://100.71.241.45:3000',
      );
    });

    test('accepts private IPv6 addresses', () {
      expect(
        normalizeLanRelayUrl('ws://[fd00::1]:3000'),
        'ws://[fd00::1]:3000',
      );
      expect(
        normalizeLanRelayUrl('ws://[fe80::1]:3000'),
        'ws://[fe80::1]:3000',
      );
    });

    test('rejects public, TLS, and path-bearing transports', () {
      expect(
        () => normalizeLanRelayUrl('ws://8.8.8.8:3000'),
        throwsFormatException,
      );
      expect(
        () => normalizeLanRelayUrl('wss://10.24.11.82:3000'),
        throwsFormatException,
      );
      expect(
        () => normalizeLanRelayUrl('ws://10.24.11.82:3000/query'),
        throwsFormatException,
      );
      expect(
        () => normalizeLanRelayUrl('ws://fcorp.example:3000'),
        throwsFormatException,
      );
    });

    test('orders private transports before the canonical relay', () {
      final config = RelayConfig(
        baseUrl: 'wss://content.example.app',
        lanRelayUrl: 'ws://10.24.11.82:3000',
      );

      expect(config.wsUrls, [
        'ws://10.24.11.82:3000',
        'wss://content.example.app',
      ]);
      expect(config.httpBaseUrls, [
        'http://10.24.11.82:3000',
        'https://content.example.app',
      ]);
    });

    test('an empty value disables the private transport', () {
      expect(normalizeLanRelayUrl('  '), isNull);
    });
  });
}
