import 'package:buzz/shared/hosted/hosted_relay_auth.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('localDeadline with a phone 120 s slow or fast', () {
    // Issuer: iat 10000, exp 10180. Sent at 1000 s, answered at once.
    const start = Duration(seconds: 1000);
    Duration deadline(int wall) => localDeadline(
      requestStart: start,
      receipt: start,
      wallAtReceipt: wall,
      exp: 10180,
      iat: 10000,
    );
    // Slow: `exp` alone would leave 300 s; the lifetime caps it at 180 s.
    expect(deadline(9880), start + const Duration(seconds: 180));
    // Fast: only the 60 s `exp` leaves.
    expect(deadline(10120), start + const Duration(seconds: 60));
    // A fractional `iat` is a valid NumericDate; the lifetime rounds down.
    const fractional = 'e30.eyJpYXQiOjEwMDAwLjUsImV4cCI6MTAxODB9.c2ln';
    final iat = HostedRelayAuth.issuedAt(fractional)!;
    expect(
      localDeadline(
        requestStart: start,
        receipt: start,
        wallAtReceipt: 9880,
        exp: 10180,
        iat: iat,
      ),
      start + const Duration(seconds: 179),
    );
    // A sleep: the wall clock jumps 1 hour, the monotonic clock doesn't.
    final wall = DateTime.utc(2026);
    final at = CachedReceipt(start, wall, const Duration(seconds: 30));
    expect(at.usable(start, wall), isTrue);
    expect(at.usable(start, wall.add(const Duration(hours: 1))), isFalse);
  });
}
