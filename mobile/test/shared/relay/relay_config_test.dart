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

  group('RelayConfig equality', () {
    test('two configs with the same origin and key are equal', () {
      // RelayConfigNotifier.build() mints a fresh instance every time the
      // active community record is re-emitted (any save of that record, e.g.
      // reserving a push-lease generation). Riverpod's default
      // updateShouldNotify is `previous != next`, so without value equality
      // every such save read as a relay change and RelaySessionNotifier
      // disconnected and reconnected.
      final a = RelayConfig(baseUrl: 'https://relay.example.com', nsec: 'nsec1abc');
      final b = RelayConfig(baseUrl: 'https://relay.example.com', nsec: 'nsec1abc');
      expect(a, equals(b));
      expect(a.hashCode, b.hashCode);
    });

    test('a different origin is not equal', () {
      final a = RelayConfig(baseUrl: 'https://relay.example.com', nsec: 'nsec1abc');
      final b = RelayConfig(baseUrl: 'https://other.example.com', nsec: 'nsec1abc');
      expect(a, isNot(equals(b)));
    });

    test('a different key is not equal', () {
      final a = RelayConfig(baseUrl: 'https://relay.example.com', nsec: 'nsec1abc');
      final b = RelayConfig(baseUrl: 'https://relay.example.com', nsec: 'nsec1xyz');
      expect(a, isNot(equals(b)));
    });
  });
}
