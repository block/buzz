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

    test('older date returns full formatted date', () {
      final older = _ts(DateTime(2026, 3, 31, 12, 0));
      expect(formatDayHeading(older, now: now), 'Tuesday, March 31, 2026');
    });

    test('midnight boundary: 11:59 PM today vs 12:01 AM tomorrow', () {
      final lateTonight = _ts(DateTime(2026, 4, 23, 23, 59));
      final earlyTomorrow = _ts(DateTime(2026, 4, 24, 0, 1));

      expect(formatDayHeading(lateTonight, now: now), 'Today');
      // Tomorrow relative to our fixed "now" is not today or yesterday.
      expect(
        formatDayHeading(earlyTomorrow, now: now),
        'Friday, April 24, 2026',
      );
    });

    test('cross-month boundary: April 1 → March 31 is yesterday', () {
      final april1 = DateTime(2026, 4, 1, 10, 0);
      final march31 = _ts(DateTime(2026, 3, 31, 20, 0));
      expect(formatDayHeading(march31, now: april1), 'Yesterday');
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
}
