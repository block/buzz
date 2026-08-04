import 'package:buzz/features/channels/message_mention_pubkeys.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  group('messageMentionPubkeys', () {
    test('plain DM messages p-tag every recipient except the sender', () {
      expect(
        messageMentionPubkeys(
          isDm: true,
          senderPubkey: 'owner',
          explicitMentions: const [],
          memberPubkeys: const ['OWNER', 'AGENT'],
          participantPubkeys: const ['owner', 'agent'],
        ),
        ['AGENT'],
      );
    });

    test('DM messages keep explicit mentions and dedupe case variants', () {
      expect(
        messageMentionPubkeys(
          isDm: true,
          senderPubkey: 'OWNER',
          explicitMentions: const ['AGENT', 'third'],
          memberPubkeys: const ['owner', 'agent'],
          participantPubkeys: const ['Owner', 'Agent', 'guest'],
        ),
        ['AGENT', 'third', 'guest'],
      );
    });

    test('stream messages keep only explicit mentions', () {
      expect(
        messageMentionPubkeys(
          isDm: false,
          senderPubkey: 'owner',
          explicitMentions: const ['agent'],
          memberPubkeys: const ['owner', 'agent', 'other'],
          participantPubkeys: const ['someone'],
        ),
        ['agent'],
      );
    });

    test('empty and self pubkeys are dropped', () {
      expect(
        messageMentionPubkeys(
          isDm: true,
          senderPubkey: 'me',
          explicitMentions: const ['', 'me'],
          memberPubkeys: const ['me', ''],
          participantPubkeys: const ['you'],
        ),
        ['you'],
      );
    });
  });
}
