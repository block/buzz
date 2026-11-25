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

  group('the stack an escaped error carries', () {
    List<String> lines(StackTrace trace) =>
        trace.toString().trimRight().split('\n');
    const gap = '<asynchronous suspension>';

    Future<StackTrace> completedWith(StackTrace callSite) async {
      final completer = Completer<void>();
      completer.completeErrorAt(Exception('late'), callSite);
      try {
        await completer.future;
      } catch (_, trace) {
        return trace;
      }
      throw StateError('the completer did not fail');
    }

    test('is cut to the innermost $maxCallSiteFrames lines', () async {
      final deep = _stackFromDepth(maxCallSiteFrames * 3);
      final deepLines = lines(deep);
      expect(deepLines.indexOf(gap), greaterThan(maxCallSiteFrames));

      final trace = await completedWith(deep);

      expect(lines(trace), [...deepLines.take(maxCallSiteFrames), gap]);
    });

    // A stack whose kept lines hold no async gap gets one after them, so
    // Sentry's parser reads it as a VM stack without first trying other
    // formats. A gap it already keeps needs none: a second one would parse as
    // an extra frame.
    for (final (gapLine, appended) in [
      (null, true),
      (30, false),
      (maxCallSiteFrames, false),
      (maxCallSiteFrames + 1, true),
      (maxCallSiteFrames + 40, true),
    ]) {
      test('with its first gap on line $gapLine '
          '${appended ? 'gets one after the cut' : 'gets no other'}', () async {
        final callSite = _TextStackTrace(
          List.generate(
            maxCallSiteFrames + 60,
            (i) => i + 1 == gapLine ? gap : '#$i frame$i (file.dart:$i)',
          ),
        );

        expect(lines(await completedWith(callSite)), [
          ...callSite.lines.take(maxCallSiteFrames),
          if (appended) gap,
        ]);
      });
    }

    test('is formatted only when read, and only once', () async {
      final callSite = _CountingStackTrace(maxCallSiteFrames + 36);

      final trace = await completedWith(callSite);
      expect(callSite.formats, 0);

      trace.toString();
      expect(lines(trace), hasLength(maxCallSiteFrames + 1));
      expect(callSite.formats, 1);
    });

    test('is kept whole when it is short', () async {
      final shallow = _stackFromDepth(3);

      expect((await completedWith(shallow)).toString(), shallow.toString());
    });
  });
}

StackTrace _stackFromDepth(int depth) =>
    depth == 0 ? StackTrace.current : _stackFromDepth(depth - 1);

final class _CountingStackTrace implements StackTrace {
  _CountingStackTrace(this._lines);

  final int _lines;
  int formats = 0;

  @override
  String toString() {
    formats++;
    return List.generate(
      _lines,
      (i) => '#$i frame$i (file.dart:$i)',
    ).join('\n');
  }
}

final class _TextStackTrace implements StackTrace {
  _TextStackTrace(this.lines);

  final List<String> lines;

  @override
  String toString() => '${lines.join('\n')}\n';
}
