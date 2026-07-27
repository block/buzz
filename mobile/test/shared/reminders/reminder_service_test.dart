import 'dart:convert';

import 'package:buzz/shared/reminders/reminder_service.dart';
import 'package:buzz/shared/reminders/reminder_time_presets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:nostr/nostr.dart' as nostr;

void main() {
  const target = ReminderTarget(
    eventId: 'event-1',
    channelId: 'channel-1',
    preview: 'hello world',
    authorPubkey: 'author-pk',
  );

  group('buildReminderPlaintext', () {
    test('matches the desktop NIP-ER content shape', () {
      final plaintext = buildReminderPlaintext(target: target, note: 'ping');
      expect(jsonDecode(plaintext), {
        'target': {
          'eventId': 'event-1',
          'channelId': 'channel-1',
          'preview': 'hello world',
          'authorPubkey': 'author-pk',
        },
        'note': 'ping',
        'status': 'pending',
      });
    });

    test('omits note when absent or empty', () {
      for (final note in [null, '']) {
        final decoded =
            jsonDecode(buildReminderPlaintext(target: target, note: note))
                as Map<String, dynamic>;
        expect(decoded.containsKey('note'), isFalse);
        expect(decoded['status'], 'pending');
      }
    });
  });

  group('randomReminderDTag', () {
    test('is 32 lowercase hex chars (128 bits)', () {
      final dTag = randomReminderDTag();
      expect(dTag, matches(RegExp(r'^[0-9a-f]{32}$')));
    });

    test('is unique across calls', () {
      expect(randomReminderDTag(), isNot(randomReminderDTag()));
    });
  });

  group('buildReminderTags', () {
    test('emits d and strict-decimal not_before tags', () {
      expect(buildReminderTags(dTag: 'abc', notBefore: 1753000000), [
        ['d', 'abc'],
        ['not_before', '1753000000'],
      ]);
    });

    test('rejects negative timestamps', () {
      expect(
        () => buildReminderTags(dTag: 'abc', notBefore: -1),
        throwsArgumentError,
      );
    });
  });

  group('ReminderCrypto', () {
    test('encrypts to self and decrypts back', () {
      final keys = nostr.Keys.generate();
      final crypto = ReminderCrypto(keys.nsec, keys.public);

      const plaintext = '{"status":"pending","note":"hi"}';
      final ciphertext = crypto.encrypt(plaintext);
      expect(ciphertext, isNot(contains('pending')));
      expect(crypto.decrypt(ciphertext), plaintext);
    });
  });

  group('reminder time presets', () {
    test('presets resolve strictly in the future', () {
      final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
      for (final preset in reminderTimePresets) {
        expect(preset.getTimestamp(), greaterThan(now), reason: preset.label);
      }
    });

    test('nextDayAt9am rolls past instants forward', () {
      final tenAm = DateTime(2026, 7, 25, 10);
      final next = nextDayAt9am(0, now: tenAm);
      expect(
        DateTime.fromMillisecondsSinceEpoch(next * 1000),
        DateTime(2026, 7, 26, 9),
      );
    });

    test('nextDayAt9am keeps future same-day instants', () {
      final eightAm = DateTime(2026, 7, 25, 8);
      final next = nextDayAt9am(0, now: eightAm);
      expect(
        DateTime.fromMillisecondsSinceEpoch(next * 1000),
        DateTime(2026, 7, 25, 9),
      );
    });

    test('daysUntilNextMonday is 1-7 and lands on Monday', () {
      for (var day = 20; day < 27; day++) {
        final now = DateTime(2026, 7, day);
        final days = daysUntilNextMonday(now);
        expect(days, inInclusiveRange(1, 7));
        expect(now.add(Duration(days: days)).weekday, DateTime.monday);
      }
    });

    test('combineCustomDateTime rejects past instants', () {
      final yesterday = DateTime.now().subtract(const Duration(days: 1));
      expect(combineCustomDateTime(yesterday, 9, 0), isNull);

      final tomorrow = DateTime.now().add(const Duration(days: 1));
      expect(combineCustomDateTime(tomorrow, 9, 0), isNotNull);
    });
  });
}
