import 'dart:async';

import 'package:buzz/features/channels/thread_replies_provider.dart';
import 'package:buzz/features/forum/forum_provider.dart';
import 'package:buzz/main.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

// ---------------------------------------------------------------------------
// Fake sessions
// ---------------------------------------------------------------------------

class _DeadlineHttpSession extends RelaySessionNotifier {
  int queryCount = 0;

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  @override
  Future<List<NostrEvent>> queryRelay(
    List<NostrFilter> filters, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    queryCount++;
    // Relay HTTP 503 with JSON body: {"error":"query timed out"}
    throw RelayException(503, '{"error":"query timed out"}');
  }
}

class _DeadlineWsSession extends RelaySessionNotifier {
  int historyCount = 0;

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    historyCount++;
    // WebSocket CLOSED: Exception("error: query timed out")
    throw Exception('error: query timed out');
  }
}

class _OrdinaryErrorSession extends RelaySessionNotifier {
  int queryCount = 0;

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  @override
  Future<List<NostrEvent>> queryRelay(
    List<NostrFilter> filters, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    queryCount++;
    throw RelayException(500, '{"error":"internal server error"}');
  }

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    queryCount++;
    throw RelayException(500, '{"error":"internal server error"}');
  }
}

// ---------------------------------------------------------------------------
// isRelayDeadlineError unit tests
// ---------------------------------------------------------------------------

void main() {
  test('the production root scope uses relayProviderRetry', () async {
    SharedPreferences.setMockInitialValues({});
    final scope = buildRootProviderScope(
      prefs: await SharedPreferences.getInstance(),
      child: const SizedBox(),
    );
    expect(scope.retry, same(relayProviderRetry));
  });

  group('isRelayDeadlineError', () {
    test('matches HTTP 503 query timed out', () {
      expect(
        isRelayDeadlineError(
          RelayException(503, '{"error":"query timed out"}'),
        ),
        isTrue,
      );
    });

    test('matches HTTP 503 with whitespace in body value', () {
      expect(
        isRelayDeadlineError(
          RelayException(503, '{"error":" Query Timed Out "}'),
        ),
        isTrue,
      );
    });

    test('does not match HTTP 503 with different error', () {
      expect(
        isRelayDeadlineError(
          RelayException(503, '{"error":"service unavailable"}'),
        ),
        isFalse,
      );
    });

    test('does not match HTTP 503 with non-JSON body', () {
      expect(isRelayDeadlineError(RelayException(503, 'plain text')), isFalse);
    });

    test('does not match HTTP 500', () {
      expect(
        isRelayDeadlineError(
          RelayException(500, '{"error":"query timed out"}'),
        ),
        isFalse,
      );
    });

    test('matches WebSocket CLOSED exception string', () {
      expect(isRelayDeadlineError(Exception('error: query timed out')), isTrue);
    });

    test('does not match ordinary exception', () {
      expect(isRelayDeadlineError(Exception('connection refused')), isFalse);
    });

    test('does not match TimeoutException', () {
      expect(
        isRelayDeadlineError(
          TimeoutException(
            'Relay history request timed out after 0:00:08.000000',
          ),
        ),
        isFalse,
      );
    });
  });

  // -------------------------------------------------------------------------
  // threadRepliesProvider — HTTP path
  // -------------------------------------------------------------------------

  group('threadRepliesProvider deadline retry suppression', () {
    const args = ThreadRepliesArgs(channelId: 'chan', rootId: 'root');

    test(
      'deadline: settles error in exactly one call and does not rebuild',
      () async {
        final session = _DeadlineHttpSession();
        final container = ProviderContainer(
          retry: relayProviderRetry,
          overrides: [relaySessionProvider.overrideWith(() => session)],
        );
        addTearDown(container.dispose);

        final sub = container.listen(threadRepliesProvider(args), (_, _) {});
        addTearDown(sub.close);

        await container.pump();
        await Future<void>.delayed(const Duration(milliseconds: 500));

        expect(container.read(threadRepliesProvider(args)).hasError, isTrue);
        expect(session.queryCount, 1);
      },
    );

    test(
      'ordinary error: retries at least once with production retry policy',
      () async {
        final session = _OrdinaryErrorSession();
        final container = ProviderContainer(
          retry: relayProviderRetry,
          overrides: [relaySessionProvider.overrideWith(() => session)],
        );
        addTearDown(container.dispose);

        final sub = container.listen(threadRepliesProvider(args), (_, _) {});
        addTearDown(sub.close);

        await container.pump();
        await Future<void>.delayed(const Duration(milliseconds: 400));

        // At least 2 calls means Riverpod issued at least one retry.
        expect(session.queryCount, greaterThanOrEqualTo(2));
      },
    );
  });

  // -------------------------------------------------------------------------
  // forumPostsProvider — WebSocket history path
  // -------------------------------------------------------------------------

  group('forumPostsProvider deadline retry suppression', () {
    test(
      'deadline: settles error in exactly one call and does not rebuild',
      () async {
        final session = _DeadlineWsSession();
        final container = ProviderContainer(
          retry: relayProviderRetry,
          overrides: [relaySessionProvider.overrideWith(() => session)],
        );
        addTearDown(container.dispose);

        final sub = container.listen(forumPostsProvider('chan'), (_, _) {});
        addTearDown(sub.close);

        await container.pump();
        await Future<void>.delayed(const Duration(milliseconds: 500));

        expect(container.read(forumPostsProvider('chan')).hasError, isTrue);
        expect(session.historyCount, 1);
      },
    );
  });
}
