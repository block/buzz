import 'package:buzz/shared/hosted/clock_hint.dart';
import 'package:buzz/shared/hosted/hosted_relay_auth.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  group('localDeadline', () {
    const start = Duration(seconds: 1000);
    const issuerIat = 10000;
    const issuerExp = 10180;

    Duration deadline({
      required int delay,
      required int wall,
      int iat = issuerIat,
    }) => localDeadline(
      requestStart: start,
      receipt: start + Duration(seconds: delay),
      wallAtReceipt: wall,
      exp: issuerExp,
      iat: iat,
    );

    test('a phone 120 s slow does not stretch the token', () {
      // `exp` alone would leave 300 s; the lifetime from the request start
      // leaves 180 s.
      expect(
        deadline(delay: 0, wall: issuerIat - 120, iat: issuerIat),
        start + const Duration(seconds: 180),
      );
    });

    test('a phone 120 s fast keeps only what exp leaves', () {
      expect(
        deadline(delay: 0, wall: issuerIat + 120, iat: issuerIat),
        start + const Duration(seconds: 60),
      );
    });

    test('a delayed issuer reply counts against the token', () {
      expect(
        deadline(delay: 20, wall: issuerIat - 100, iat: issuerIat),
        start + const Duration(seconds: 180),
      );
    });

    test('a short session-capped token keeps its short life', () {
      expect(
        localDeadline(
          requestStart: start,
          receipt: start,
          wallAtReceipt: issuerIat,
          exp: issuerIat + 30,
          iat: issuerIat,
        ),
        start + const Duration(seconds: 30),
      );
    });

    test('no budget left gives the receipt time', () {
      expect(
        deadline(delay: 5, wall: issuerExp + 1, iat: issuerIat),
        start + const Duration(seconds: 5),
      );
    });
  });

  group('clockHintMinutes', () {
    final now = DateTime.utc(2026, 11, 13, 12);
    String date(Duration offset) {
      final t = now.add(offset);
      const days = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'];
      const months = [
        'Jan',
        'Feb',
        'Mar',
        'Apr',
        'May',
        'Jun',
        'Jul',
        'Aug',
        'Sep',
        'Oct',
        'Nov',
        'Dec',
      ];
      String two(int n) => n.toString().padLeft(2, '0');
      return '${days[t.weekday - 1]}, ${two(t.day)} ${months[t.month - 1]} '
          '${t.year} ${two(t.hour)}:${two(t.minute)}:${two(t.second)} GMT';
    }

    test('hints when Date is 130 s off in either direction', () {
      expect(clockHintMinutes(date(const Duration(seconds: 130)), now), 2);
      expect(clockHintMinutes(date(const Duration(seconds: -130)), now), 2);
    });

    test('no hint within 115 s, at 116–120 s it hints', () {
      expect(clockHintMinutes(date(const Duration(seconds: 3)), now), isNull);
      expect(clockHintMinutes(date(const Duration(seconds: 115)), now), isNull);
      expect(clockHintMinutes(date(const Duration(seconds: 118)), now), 2);
    });

    test('no hint without a readable Date', () {
      expect(clockHintMinutes(null, now), isNull);
      expect(clockHintMinutes('not a date', now), isNull);
    });

    test('copy', () {
      expect(
        clockHintText(2),
        "Your device's clock looks about 2 minutes off, which can cause "
        'this. Check your date and time settings.',
      );
    });
  });
}
