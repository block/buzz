import 'dart:convert';

import 'package:buzz/features/activity/reminders_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';

NostrEvent reminderEvent({
  List<List<String>> tags = const [],
  String content = 'ciphertext',
  int createdAt = 100,
}) => NostrEvent(
  id: 'ev1',
  pubkey: 'pk1',
  createdAt: createdAt,
  kind: kindEventReminder,
  tags: tags,
  content: content,
  sig: '',
);

void main() {
  group('parseNotBefore', () {
    test('accepts plain unix seconds', () {
      expect(parseNotBefore('1753400000'), 1753400000);
      expect(parseNotBefore('0'), 0);
    });

    test('rejects malformed values', () {
      expect(parseNotBefore('01'), isNull);
      expect(parseNotBefore('-5'), isNull);
      expect(parseNotBefore('12.5'), isNull);
      expect(parseNotBefore('abc'), isNull);
      expect(parseNotBefore(''), isNull);
    });
  });

  group('parseReminderContent', () {
    test('parses a well-formed reminder with target', () {
      final parsed = parseReminderContent(
        jsonEncode({
          'status': 'pending',
          'note': 'follow up',
          'target': {
            'eventId': 'ev1',
            'channelId': 'ch1',
            'preview': 'hello',
            'authorPubkey': 'pk1',
          },
        }),
      );
      expect(parsed, isNotNull);
      expect(parsed!.status, 'pending');
      expect(parsed.note, 'follow up');
      expect(parsed.target?.eventId, 'ev1');
      expect(parsed.target?.channelId, 'ch1');
    });

    test('accepts a note-only reminder', () {
      final parsed = parseReminderContent(
        jsonEncode({'status': 'pending', 'note': 'water plants'}),
      );
      expect(parsed?.note, 'water plants');
      expect(parsed?.target, isNull);
    });

    test('fails closed on off-shape content', () {
      expect(parseReminderContent('not json'), isNull);
      expect(parseReminderContent('[]'), isNull);
      expect(parseReminderContent(jsonEncode({'status': 'snoozed'})), isNull);
      // No target and no note is meaningless.
      expect(parseReminderContent(jsonEncode({'status': 'pending'})), isNull);
      // Target missing required fields.
      expect(
        parseReminderContent(
          jsonEncode({
            'status': 'pending',
            'target': {'eventId': 'ev1'},
          }),
        ),
        isNull,
      );
    });
  });

  group('decodeReminderEvent', () {
    final plaintext = jsonEncode({'status': 'pending', 'note': 'hi'});

    test('decodes d-tag, not_before, and decrypted content', () {
      final reminder = decodeReminderEvent(
        reminderEvent(
          tags: [
            ['d', 'rem1'],
            ['not_before', '1753400000'],
          ],
        ),
        decrypt: (_) => plaintext,
      );
      expect(reminder, isNotNull);
      expect(reminder!.id, 'rem1');
      expect(reminder.notBefore, 1753400000);
      expect(reminder.status, 'pending');
      expect(reminder.isDue(1753400001), isTrue);
      expect(reminder.isDue(1753399999), isFalse);
    });

    test('requires a d tag', () {
      expect(
        decodeReminderEvent(reminderEvent(), decrypt: (_) => plaintext),
        isNull,
      );
    });

    test('fails closed when decryption throws', () {
      expect(
        decodeReminderEvent(
          reminderEvent(
            tags: [
              ['d', 'rem1'],
            ],
          ),
          decrypt: (_) => throw StateError('bad ciphertext'),
        ),
        isNull,
      );
    });
  });
}
