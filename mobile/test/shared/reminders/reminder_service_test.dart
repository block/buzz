import 'dart:async';
import 'dart:convert';

import 'package:buzz/shared/relay/relay.dart';
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

  group('buildReminderPlaintext without a target', () {
    test('writes only the note and status, with no target key', () {
      expect(jsonDecode(buildReminderPlaintext(note: 'Call the dentist')), {
        'note': 'Call the dentist',
        'status': 'pending',
      });
    });
  });

  group('text reminders', () {
    late nostr.Keys keys;
    late _RecordingRelay relay;
    late ReminderCrypto crypto;
    late ReminderService service;

    setUp(() {
      keys = nostr.Keys.generate();
      relay = _RecordingRelay(keys.nsec);
      crypto = ReminderCrypto(keys.nsec, keys.public);
      service = ReminderService(signedEventRelay: relay, crypto: crypto);
    });

    Map<String, dynamic> content() =>
        jsonDecode(crypto.decrypt(relay.submitted.single.content))
            as Map<String, dynamic>;

    test('createTextReminder writes a trimmed note with no target', () async {
      await service.createTextReminder(
        text: '  Call the dentist\nBook a cleaning  ',
        notBefore: 1753000000,
      );

      final event = relay.submitted.single;
      expect(event.kind, EventKind.eventReminder);
      expect(content(), {
        'note': 'Call the dentist\nBook a cleaning',
        'status': 'pending',
      });
      expect(event.tags, [
        ['d', matches(RegExp(r'^[0-9a-f]{32}$'))],
        ['not_before', '1753000000'],
      ]);
    });

    test('createTextReminder requires text', () async {
      for (final blank in ['', '   ', '\n\t']) {
        await expectLater(
          service.createTextReminder(text: blank, notBefore: 1753000000),
          throwsArgumentError,
        );
      }
      expect(relay.submitted, isEmpty);
    });

    test('createTextReminder caps the text length', () async {
      await service.createTextReminder(
        text: 'a' * maxReminderNoteLength,
        notBefore: 1753000000,
      );
      await expectLater(
        service.createTextReminder(
          text: 'a' * (maxReminderNoteLength + 1),
          notBefore: 1753000000,
        ),
        throwsArgumentError,
      );
      expect(relay.submitted, hasLength(1));
    });

    test('editing a message reminder keeps its d tag, target, status and '
        'time', () async {
      await service.updateReminderNote(
        dTag: 'reminder-d1',
        notBefore: 1753000000,
        target: target,
        note: '  Reply after standup  ',
      );

      expect(content(), {
        'target': target.toJson(),
        'note': 'Reply after standup',
        'status': 'pending',
      });
      expect(relay.submitted.single.tags, [
        ['d', 'reminder-d1'],
        ['not_before', '1753000000'],
      ]);
    });

    test('clearing a message reminder note omits the note key', () async {
      for (final cleared in ['', '   ', null]) {
        relay.submitted.clear();
        await service.updateReminderNote(
          dTag: 'reminder-d1',
          notBefore: 1753000000,
          target: target,
          note: cleared,
        );

        final decoded = content();
        expect(decoded.containsKey('note'), isFalse);
        expect(decoded['target'], target.toJson());
        expect(decoded['status'], 'pending');
      }
    });

    test(
      'editing a written reminder keeps it target-free and needs text',
      () async {
        await service.updateReminderNote(
          dTag: 'reminder-d2',
          notBefore: 1753000000,
          note: 'New wording',
        );
        expect(content(), {'note': 'New wording', 'status': 'pending'});
        expect(relay.submitted.single.tags.first, ['d', 'reminder-d2']);

        await expectLater(
          service.updateReminderNote(
            dTag: 'reminder-d2',
            notBefore: 1753000000,
            note: '  ',
          ),
          throwsArgumentError,
        );
        expect(relay.submitted, hasLength(1));
      },
    );
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

/// Records submitted events instead of signing and publishing them.
class _RecordingRelay extends SignedEventRelay {
  _RecordingRelay(String nsec)
    : super(session: RelaySessionNotifier(), nsec: nsec);

  final submitted = <({int kind, String content, List<List<String>> tags})>[];

  @override
  Future<NostrEvent> submit({
    required int kind,
    required String content,
    required List<List<String>> tags,
    int? createdAt,
    FutureOr<void> Function(NostrEvent event)? onSigned,
    Future<bool> Function()? mayPublish,
  }) async {
    submitted.add((kind: kind, content: content, tags: tags));
    return NostrEvent(
      id: 'event-${submitted.length}',
      pubkey: '',
      createdAt: 0,
      kind: kind,
      tags: tags,
      content: '',
      sig: '',
    );
  }
}
