import 'dart:async';

import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  group('the reason class of a relay message', () {
    const cases = {
      'invalid: bad signature': RelayReason.invalid,
      'rate-limited: quota exceeded; retry in 4s': RelayReason.rateLimited,
      'restricted: access revoked': RelayReason.restricted,
      'auth-required: verification failed': RelayReason.authRequired,
      'blocked: community policy': RelayReason.blocked,
      'duplicate: already have this event': RelayReason.duplicate,
      'pow: insufficient work': RelayReason.pow,
      'unsupported: filter extension': RelayReason.unsupported,
      'error: query timed out': RelayReason.error,
      'INVALID: shouted': RelayReason.invalid,
      '  invalid : padded': RelayReason.invalid,
      'invalid:': RelayReason.invalid,
      'timeout: not a relay prefix': RelayReason.other,
      'other: not a relay prefix': RelayReason.other,
      'teapot: unknown prefix': RelayReason.other,
      'no prefix at all': RelayReason.other,
      'a sentence, then invalid: later': RelayReason.other,
      '': RelayReason.other,
    };
    for (final entry in cases.entries) {
      test('"${entry.key}" is ${entry.value.name}', () {
        expect(RelayReason.fromMessage(entry.key), entry.value);
      });
    }

    test('the wording that leaves the device is the relay prefix', () {
      expect(
        {for (final reason in RelayReason.values) reason.tag},
        {
          'invalid',
          'rate-limited',
          'restricted',
          'auth-required',
          'blocked',
          'duplicate',
          'pow',
          'unsupported',
          'error',
          'timeout',
          'other',
        },
      );
    });
  });

  group('the reason class of a /query status', () {
    const cases = {
      429: RelayReason.rateLimited,
      401: RelayReason.authRequired,
      403: RelayReason.restricted,
      500: RelayReason.error,
      503: RelayReason.error,
      599: RelayReason.error,
      400: RelayReason.other,
      404: RelayReason.other,
      302: RelayReason.other,
      600: RelayReason.other,
    };
    for (final entry in cases.entries) {
      test('${entry.key} is ${entry.value.name}', () {
        expect(RelayReason.fromHttpStatus(entry.key), entry.value);
      });
    }
  });

  group('what a relay error says about itself', () {
    test('a refused query is a relay exception that names the query', () {
      final error = RelayQueryException(429, '{"error":"slow down"}');

      expect(error, isA<RelayException>());
      expect(error.call, RelayCall.query);
      expect(error.reason, RelayReason.rateLimited);
      expect(error.statusCode, 429);
      expect(error.toString(), 'RelayException(429): {"error":"slow down"}');
    });

    test('a rejected publish names the publish and the relay prefix', () {
      const error = RelayEventRejectedException('blocked: secret reason');

      expect(error.call, RelayCall.publish);
      expect(error.reason, RelayReason.blocked);
      expect(error.toString(), 'Exception: blocked: secret reason');
    });

    test('a closed subscription names its call and keeps its text', () {
      final error = RelayClosedException(
        RelayCall.history,
        'error: query timed out',
      );

      expect(error.call, RelayCall.history);
      expect(error.reason, RelayReason.error);
      expect(error.toString(), 'Exception: error: query timed out');
      expect(isRelayDeadlineError(error), isTrue);
      expect(relayErrorMessage(error), 'Query timed out');
    });

    test('an access refusal is still recognized as one', () {
      final error = RelayClosedException(
        RelayCall.subscribe,
        'restricted: not a member',
      );

      expect(isRelayAccessDenied(error), isTrue);
    });

    test('a relay timeout is a timeout of its call', () {
      final error = RelayTimeoutException(RelayCall.query, 'slow');

      expect(error, isA<TimeoutException>());
      expect(error.call, RelayCall.query);
      expect(error.reason, RelayReason.timeout);
    });

    test('an unacknowledged publish is still a timeout, of the publish', () {
      final error = RelayAckTimeoutError('no OK', const Duration(seconds: 8));

      expect(error, isA<TimeoutException>());
      expect(error, isA<RelayTimeoutException>());
      expect(error.call, RelayCall.publish);
      expect(error.reason, RelayReason.timeout);
      expect(error.duration, const Duration(seconds: 8));
    });
  });
}
