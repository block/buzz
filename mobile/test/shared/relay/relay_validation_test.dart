import 'package:flutter_test/flutter_test.dart';

import 'package:buzz/shared/relay/relay_validation.dart';

void main() {
  group('validateInviteRelayUri', () {
    test('accepts public secure relay origins', () {
      for (final url in [
        'wss://relay.example.com',
        'wss://relay.example.com/',
        'wss://relay.example.com:8443',
        'wss://8.8.8.8',
        'wss://[2001:4860:4860::8888]',
        'wss://[2001:1::1]',
        'wss://[2001:1::2]',
        'wss://[2001:1::3]',
        'wss://[2001:3::1]',
        'wss://[2001:4:112::1]',
        'wss://[2001:20::1]',
        'wss://[2001:30::1]',
        'wss://[2001:200::1]',
        'wss://[2620:4f:8000::1]',
        'wss://[3fff:1000::1]',
        'wss://[2606:4700:4700::1111]',
      ]) {
        expect(
          () => validateInviteRelayUri(
            Uri.parse(url),
            allowInsecureLocalhost: false,
          ),
          returnsNormally,
          reason: url,
        );
      }
    });

    test('allows plaintext localhost only when explicitly enabled', () {
      for (final url in ['ws://localhost:3000', 'ws://relay.localhost:3000']) {
        expect(
          () => validateInviteRelayUri(
            Uri.parse(url),
            allowInsecureLocalhost: true,
          ),
          returnsNormally,
          reason: url,
        );
        expect(
          () => validateInviteRelayUri(
            Uri.parse(url),
            allowInsecureLocalhost: false,
          ),
          throwsFormatException,
          reason: url,
        );
      }
    });

    test('rejects plaintext public relays', () {
      expect(
        () => validateInviteRelayUri(
          Uri.parse('ws://relay.example.com'),
          allowInsecureLocalhost: false,
        ),
        throwsFormatException,
      );
    });

    test('rejects non-public IPv4 literals', () {
      for (final host in [
        '0.0.0.0',
        '10.0.0.1',
        '100.64.0.1',
        '127.0.0.1',
        '169.254.169.254',
        '172.16.0.1',
        '192.168.0.1',
        '198.18.0.1',
        '224.0.0.1',
      ]) {
        expect(
          () => validateInviteRelayUri(
            Uri.parse('wss://$host'),
            allowInsecureLocalhost: false,
          ),
          throwsFormatException,
          reason: host,
        );
      }
    });

    test('rejects non-public IPv6 literals and mapped IPv4', () {
      for (final host in [
        '[::]',
        '[::1]',
        '[::ffff:127.0.0.1]',
        '[::ffff:169.254.169.254]',
        '[::ffff:a9fe:a9fe]',
        '[64:ff9b::a9fe:a9fe]',
        '[100::1]',
        '[100:0:0:1::1]',
        '[2001::1]',
        '[2001:1::4]',
        '[2001:2::1]',
        '[2001:4:111::1]',
        '[2001:10::1]',
        '[2001:40::1]',
        '[2001:db8::1]',
        '[2002:a9fe:a9fe::1]',
        '[3fff::1]',
        '[3fff:fff::1]',
        '[5f00::1]',
        '[fc00::1]',
        '[fd00::1]',
        '[fe80::1]',
        '[fec0::1]',
        '[ff02::1]',
      ]) {
        expect(
          () => validateInviteRelayUri(
            Uri.parse('wss://$host'),
            allowInsecureLocalhost: false,
          ),
          throwsFormatException,
          reason: host,
        );
      }
    });

    test('rejects legacy numeric IPv4 spellings', () {
      for (final host in ['2130706433', '0x7f000001', '0177.0.0.1', '127.1']) {
        expect(
          () => validateInviteRelayUri(
            Uri.parse('wss://$host'),
            allowInsecureLocalhost: false,
          ),
          throwsFormatException,
          reason: host,
        );
      }
    });

    test('rejects non-origin URLs', () {
      for (final url in [
        'https://relay.example.com',
        'wss://user@relay.example.com',
        'wss://relay.example.com/path',
        'wss://relay.example.com?query=x',
        'wss://relay.example.com#fragment',
      ]) {
        expect(
          () => validateInviteRelayUri(
            Uri.parse(url),
            allowInsecureLocalhost: false,
          ),
          throwsFormatException,
          reason: url,
        );
      }
    });
  });
}
