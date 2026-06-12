import 'package:flutter_test/flutter_test.dart';
import 'package:buzz/features/channels/read_state/read_state_time.dart';

void main() {
  test('dateTimeToUnixSeconds converts DateTime values and nulls', () {
    final value = DateTime.utc(2026, 4, 29, 4, 30, 45, 900);

    expect(dateTimeToUnixSeconds(value), 1777437045);
    expect(dateTimeToUnixSeconds(null), isNull);
  });

  test('isoToUnixSeconds parses stored ISO timestamps defensively', () {
    expect(isoToUnixSeconds('1970-01-01T00:00:42.000Z'), 42);
    expect(isoToUnixSeconds(''), isNull);
    expect(isoToUnixSeconds('not-a-date'), isNull);
    expect(isoToUnixSeconds(42), isNull);
  });

  test('unixSecondsToDateTime writes UTC ISO-compatible values', () {
    expect(
      unixSecondsToDateTime(1700000000).toIso8601String(),
      '2023-11-14T22:13:20.000Z',
    );
  });
}
