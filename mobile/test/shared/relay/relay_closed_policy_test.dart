import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  group('classifyRelayClosed', () {
    const cases = {
      'rate-limited: quota exceeded; retry in 4s': RelayClosedClass.rateLimited,
      'rate-limited: too many concurrent requests':
          RelayClosedClass.rateLimited,
      'restricted: access revoked': RelayClosedClass.terminal,
      'auth-required: verification failed': RelayClosedClass.terminal,
      'blocked: community policy': RelayClosedClass.terminal,
      'invalid: malformed filter': RelayClosedClass.terminal,
      'pow: insufficient work': RelayClosedClass.terminal,
      'duplicate: subscription already exists': RelayClosedClass.terminal,
      'unsupported: filter extension': RelayClosedClass.terminal,
      'error: mixed search and channel filter': RelayClosedClass.terminal,
      'error: too many subscriptions': RelayClosedClass.terminal,
      'error: relay temporarily unavailable': RelayClosedClass.retryable,
      'subscription closed by relay': RelayClosedClass.retryable,
    };

    for (final entry in cases.entries) {
      test(entry.key, () {
        expect(classifyRelayClosed(entry.key), entry.value);
      });
    }
  });

  test('parseRateLimitRetrySeconds handles canonical and absent hints', () {
    expect(
      parseRateLimitRetrySeconds('rate-limited: quota exceeded; retry in 17s'),
      17,
    );
    expect(parseRateLimitRetrySeconds('RETRY IN 2S'), 2);
    expect(parseRateLimitRetrySeconds('retry in 0s'), 0);
    expect(parseRateLimitRetrySeconds('retry in 999999s'), 999999);
    final oversizedHint = List.filled(1000, '9').join();
    expect(parseRateLimitRetrySeconds('retry in ${oversizedHint}s'), isNull);
    expect(
      parseRateLimitRetrySeconds('rate-limited: too many concurrent requests'),
      isNull,
    );
  });
}
