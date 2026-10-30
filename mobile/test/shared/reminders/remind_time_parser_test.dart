import 'package:buzz/shared/reminders/remind_time_parser.dart';
import 'package:flutter_test/flutter_test.dart';

/// America/Chicago for 2026: CDT (UTC-5) from 2026-03-08 02:00 local until
/// 2026-11-01 02:00 local, CST (UTC-6) otherwise. A fixed zone keeps the DST
/// vectors independent of the `TZ` the test process runs under.
class _Chicago2026 implements RemindZone {
  static final _dstStartUtc = DateTime.utc(2026, 3, 8, 8);
  static final _dstEndUtc = DateTime.utc(2026, 11, 1, 7);

  @override
  DateTime instantToWall(DateTime instant) {
    final utc = instant.toUtc();
    final dst = !utc.isBefore(_dstStartUtc) && utc.isBefore(_dstEndUtc);
    return utc.subtract(Duration(hours: dst ? 5 : 6));
  }

  @override
  DateTime wallToInstant(int year, int month, int day, int hour, int minute) {
    final wall = DateTime.utc(year, month, day, hour, minute);
    final dst =
        !wall.isBefore(DateTime.utc(2026, 3, 8, 3)) &&
        wall.isBefore(DateTime.utc(2026, 11, 1, 1));
    return wall.add(Duration(hours: dst ? 5 : 6));
  }
}

final _zone = _Chicago2026();

DateTime _wall(String local) {
  final parts = RegExp(r'\d+').allMatches(local).map((m) => int.parse(m[0]!));
  final [y, mo, d, h, mi] = parts.toList();
  return _zone.wallToInstant(y, mo, d, h, mi);
}

String _local(DateTime instant) {
  final wall = _zone.instantToWall(instant);
  String two(int n) => n.toString().padLeft(2, '0');
  return '${wall.year}-${two(wall.month)}-${two(wall.day)} '
      '${two(wall.hour)}:${two(wall.minute)}';
}

void main() {
  final now = _wall('2026-10-28 10:00');

  test('the fixed zone matches the spec calendar', () {
    expect(_local(now), '2026-10-28 10:00');
    expect(_zone.instantToWall(now).weekday, DateTime.wednesday);
    final crossing = _wall('2026-11-04 10:00');
    expect(
      crossing.difference(now),
      const Duration(days: 7, hours: 1),
      reason: 'DST ends between the two dates, so a week is 169 hours',
    );
  });

  const vectors = <(String, String?)>[
    ('in 20 minutes', '2026-10-28 10:20'),
    ('in an hour', '2026-10-28 11:00'),
    ('in 2 hrs', '2026-10-28 12:00'),
    ('in 3 days', '2026-10-31 10:00'),
    ('in 1 week', '2026-11-04 10:00'),
    ('at 3pm', '2026-10-28 15:00'),
    ('at 3:30 pm', '2026-10-28 15:30'),
    ('at 15:00', '2026-10-28 15:00'),
    ('at 9am', '2026-10-29 09:00'),
    ('at 10am', '2026-10-29 10:00'),
    ('at noon', '2026-10-28 12:00'),
    ('at midnight', '2026-10-29 00:00'),
    ('tomorrow', '2026-10-29 09:00'),
    ('tomorrow at 4pm', '2026-10-29 16:00'),
    ('on friday', '2026-10-30 09:00'),
    ('Friday at 2pm', '2026-10-30 14:00'),
    ('on wednesday', '2026-11-04 09:00'),
    ('next monday', '2026-11-02 09:00'),
    ('next week', '2026-11-02 09:00'),
    ('at 3', null),
    ('at 25:00', null),
    ('at 13pm', null),
    ('in 0 minutes', null),
    ('in 400 days', null),
    ('yesterday', null),
  ];

  test('<when> vectors, as the end and as the start of the command', () {
    for (final (phrase, due) in vectors) {
      for (final command in [
        'me to stretch $phrase',
        'me $phrase to stretch',
      ]) {
        final result = parseRemindCommand(command, now: now, zone: _zone);
        if (due == null) {
          expect(result, isA<RemindRejected>(), reason: command);
          continue;
        }
        expect(result, isA<RemindParsed>(), reason: command);
        result as RemindParsed;
        expect(_local(result.dueAt), due, reason: command);
        expect(result.note, 'stretch', reason: command);
      }
    }
  });

  test('exact-duration units are absolute, calendar units keep local time', () {
    final hour = parseRemindCommand(
      'me to x in 168 hours',
      now: now,
      zone: _zone,
    );
    final week = parseRemindCommand('me to x in 1 week', now: now, zone: _zone);
    expect(_local((hour as RemindParsed).dueAt), '2026-11-04 09:00');
    expect(_local((week as RemindParsed).dueAt), '2026-11-04 10:00');
  });

  const commands = <(String, String?, String?, RemindParseError?)>[
    (
      'me to submit the report in 20 minutes',
      'submit the report',
      '2026-10-28 10:20',
      null,
    ),
    ('me tomorrow at 4pm to call Sam', 'call Sam', '2026-10-29 16:00', null),
    (
      'me check the deploy at 3pm',
      'check the deploy',
      '2026-10-28 15:00',
      null,
    ),
    ('ME To Stretch IN 5 MINUTES', 'Stretch', '2026-10-28 10:05', null),
    ('me in 2 hours', null, null, RemindParseError.missingNote),
    ('me to in 2 hours', null, null, RemindParseError.missingNote),
    ('me to stretch', null, null, RemindParseError.missingTime),
    ('me to meet at 3', null, null, RemindParseError.ambiguousHour),
    ('me to meet at 25:00', null, null, RemindParseError.invalidTime),
    ('me to meet in 400 days', null, null, RemindParseError.outOfRange),
    ('me', null, null, RemindParseError.usage),
    ('', null, null, RemindParseError.usage),
    ('@sam to stretch in 1 hour', null, null, RemindParseError.notPersonal),
    ('to stretch in 1 hour', null, null, RemindParseError.notPersonal),
    ('meeting notes in 1 hour', null, null, RemindParseError.notPersonal),
  ];

  test('full commands', () {
    for (final (command, note, due, error) in commands) {
      final result = parseRemindCommand(command, now: now, zone: _zone);
      switch (result) {
        case RemindParsed():
          expect(error, isNull, reason: command);
          expect((result.note, _local(result.dueAt)), (note, due));
        case RemindRejected():
          expect(result.error, error, reason: command);
      }
    }
  });

  test('the due time is after now and at most 365 days ahead', () {
    for (final (command, ok) in [
      ('me to x at 10am', true),
      ('me to x in 364 days', true),
      ('me to x in 366 days', false),
    ]) {
      final result = parseRemindCommand(command, now: now, zone: _zone);
      expect(result is RemindParsed, ok, reason: command);
      if (result is RemindParsed) {
        expect(result.dueAt.isAfter(now), isTrue, reason: command);
      }
    }
  });

  test('the device zone keeps wall-clock time for calendar units', () {
    final local = DateTime(2026, 10, 28, 10);
    final result = parseRemindCommand('me to x in 3 days', now: local);
    expect((result as RemindParsed).dueAt, DateTime(2026, 10, 31, 10));
  });
}
