import 'package:buzz/shared/community/community.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  group('Community.nameFromUrl', () {
    test('skips service labels ahead of the org label', () {
      expect(Community.nameFromUrl('wss://buzz.nilor.cool'), 'nilor');
      expect(Community.nameFromUrl('wss://relay.example.com'), 'example');
      expect(Community.nameFromUrl('wss://buzz.relay.example.com'), 'example');
    });

    test('keeps the registrable domain label', () {
      // The service label IS the org when nothing follows it but the TLD.
      expect(Community.nameFromUrl('wss://buzz.example.com'), 'example');
      expect(Community.nameFromUrl('wss://acme.example.com'), 'acme');
      // Two-label hosts keep the full host, matching existing behavior.
      expect(Community.nameFromUrl('wss://nilor.cool'), 'nilor.cool');
    });

    test('preserves local and fallback special cases', () {
      expect(Community.nameFromUrl('ws://localhost:3000'), 'Local Dev');
      expect(Community.nameFromUrl('ws://127.0.0.1:3000'), 'Local Dev');
    });
  });
}
