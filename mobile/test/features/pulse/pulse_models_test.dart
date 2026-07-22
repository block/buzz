import 'package:flutter_test/flutter_test.dart';
import 'package:buzz/features/pulse/pulse_models.dart';
import 'package:buzz/features/pulse/pulse_provider.dart';
import 'package:buzz/shared/relay/nostr_models.dart';

NostrEvent _contactList(List<List<String>> tags) => NostrEvent(
  id: 'event-id',
  pubkey: 'author',
  createdAt: 0,
  kind: 3,
  tags: tags,
  content: '',
  sig: '',
);

void main() {
  group('ContactEntry.toTag', () {
    test('keeps petname in the NIP-02 petname slot without a relay url', () {
      const entry = ContactEntry(pubkey: 'DEADBEEF', petname: 'alice');

      expect(entry.toTag(), ['p', 'deadbeef', '', 'alice']);
    });

    test('emits relay url and petname in order', () {
      const entry = ContactEntry(
        pubkey: 'deadbeef',
        relayUrl: 'wss://relay.example.com',
        petname: 'alice',
      );

      expect(entry.toTag(), [
        'p',
        'deadbeef',
        'wss://relay.example.com',
        'alice',
      ]);
    });

    test(
      'emits empty trailing slots when relay url and petname are absent',
      () {
        const entry = ContactEntry(pubkey: 'deadbeef');

        expect(entry.toTag(), ['p', 'deadbeef', '', '']);
      },
    );

    test('round-trips a petname through the contact list parser', () {
      const entry = ContactEntry(pubkey: 'deadbeef', petname: 'alice');

      final parsed = contactsFromEvents([
        _contactList([entry.toTag()]),
      ]);

      expect(parsed.single.pubkey, 'deadbeef');
      expect(parsed.single.relayUrl, isNull);
      expect(parsed.single.petname, 'alice');
    });
  });
}
