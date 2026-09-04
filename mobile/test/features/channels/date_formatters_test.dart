import 'package:flutter_test/flutter_test.dart';
import 'package:buzz/features/channels/date_formatters.dart';

/// Helper: build a unix-second timestamp from a local DateTime.
int _ts(DateTime local) => local.millisecondsSinceEpoch ~/ 1000;

void main() {
  group('formatDayHeading', () {
    // Fix "now" so tests are deterministic.
    final now = DateTime(2026, 4, 23, 14, 30); // Apr 23 2026, 2:30 PM local

    test('same day returns "Today"', () {
      final morning = _ts(DateTime(2026, 4, 23, 8, 0));
      expect(formatDayHeading(morning, now: now), 'Today');
    });

    test('yesterday returns "Yesterday"', () {
      final yesterday = _ts(DateTime(2026, 4, 22, 18, 0));
      expect(formatDayHeading(yesterday, now: now), 'Yesterday');
    });

    test('two to six days back names the weekday alone', () {
      expect(
        formatDayHeading(_ts(DateTime(2026, 4, 21, 9)), now: now),
        'Tuesday',
      );
      expect(
        formatDayHeading(_ts(DateTime(2026, 4, 18, 9)), now: now),
        'Saturday',
      );
    });

    test(
      'a week back switches to weekday + date, without the current year',
      () {
        // Seven days is the first day a bare weekday name stops being
        // unambiguous, so the date joins it.
        expect(
          formatDayHeading(_ts(DateTime(2026, 4, 16, 9)), now: now),
          'Thursday, April 16',
        );
        expect(
          formatDayHeading(_ts(DateTime(2026, 3, 31, 12)), now: now),
          'Tuesday, March 31',
        );
      },
    );

    test('an earlier year keeps the year', () {
      expect(
        formatDayHeading(_ts(DateTime(2025, 3, 31, 12)), now: now),
        'March 31, 2025',
      );
    });

    test('no divider carries an ordinal suffix', () {
      // 1/2/3 and the 11/12/13 exceptions are where ordinals used to appear.
      for (final day in [1, 2, 3, 11, 12, 13, 21, 22, 23, 31]) {
        expect(
          formatDayHeading(_ts(DateTime(2025, 1, day, 12)), now: now),
          isNot(matches(RegExp(r'\d(st|nd|rd|th)'))),
        );
      }
    });

    test('the oldest band still tells consecutive days apart', () {
      // The reason the day is kept rather than collapsed to "Jan 2022".
      final labels = [3, 4, 5]
          .map(
            (day) =>
                formatDayHeading(_ts(DateTime(2022, 1, day, 12)), now: now),
          )
          .toSet();
      expect(labels, hasLength(3));
    });

    test('midnight boundary: 11:59 PM today vs 12:01 AM tomorrow', () {
      final lateTonight = _ts(DateTime(2026, 4, 23, 23, 59));
      final earlyTomorrow = _ts(DateTime(2026, 4, 24, 0, 1));

      expect(formatDayHeading(lateTonight, now: now), 'Today');
      // A future timestamp must not read as the recent past. The attached
      // date keeps the weekday from being mistaken for last Friday.
      expect(formatDayHeading(earlyTomorrow, now: now), 'Friday, April 24');
    });

    test('cross-month boundary: April 1 → March 31 is yesterday', () {
      final april1 = DateTime(2026, 4, 1, 10, 0);
      final march31 = _ts(DateTime(2026, 3, 31, 20, 0));
      expect(formatDayHeading(march31, now: april1), 'Yesterday');
    });

    test('bands are calendar days, not 24-hour windows', () {
      // 15 hours old but across midnight, so "Yesterday"; 22 hours old on the
      // same calendar day, so "Today".
      final elevenPmYesterday = DateTime(2026, 4, 23, 2, 0);
      expect(
        formatDayHeading(
          _ts(DateTime(2026, 4, 22, 23, 0)),
          now: elevenPmYesterday,
        ),
        'Yesterday',
      );
      expect(
        formatDayHeading(
          _ts(DateTime(2026, 4, 23, 1, 0)),
          now: DateTime(2026, 4, 23, 23, 0),
        ),
        'Today',
      );
    });
  });

  group('isSameDay', () {
    test('same calendar day returns true', () {
      final a = _ts(DateTime(2026, 4, 23, 1, 0));
      final b = _ts(DateTime(2026, 4, 23, 23, 59));
      expect(isSameDay(a, b), isTrue);
    });

    test('different days returns false', () {
      final a = _ts(DateTime(2026, 4, 23, 23, 59));
      final b = _ts(DateTime(2026, 4, 24, 0, 1));
      expect(isSameDay(a, b), isFalse);
    });
  });

  group('formatThreadSummaryLastReplyTime', () {
    const now = 2_000_000;

    test('uses expanded relative units', () {
      expect(
        formatThreadSummaryLastReplyTime(now - 30, nowSeconds: now),
        'just now',
      );
      expect(
        formatThreadSummaryLastReplyTime(now - 60, nowSeconds: now),
        '1 minute ago',
      );
      expect(
        formatThreadSummaryLastReplyTime(now - 3 * 3600, nowSeconds: now),
        '3 hours ago',
      );
      expect(
        formatThreadSummaryLastReplyTime(now - 2 * 86400, nowSeconds: now),
        '2 days ago',
      );
    });

    test('dates older replies with a short month and no ordinal', () {
      final may19 = _ts(DateTime(2026, 5, 19, 12));
      final may1 = _ts(DateTime(2026, 5, 1, 12));

      expect(
        formatThreadSummaryLastReplyTime(
          may19,
          nowSeconds: _ts(DateTime(2026, 5, 27, 12)),
        ),
        'on May 19',
      );
      // The 1st is where an ordinal is most tempting.
      expect(
        formatThreadSummaryLastReplyTime(
          may1,
          nowSeconds: _ts(DateTime(2026, 5, 27, 12)),
        ),
        'on May 1',
      );
    });
  });
}
