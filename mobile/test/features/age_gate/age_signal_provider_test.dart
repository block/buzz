import 'dart:async';

import 'package:buzz/features/age_gate/age_signal_provider.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  tearDown(() {
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(ageSignalChannel, null);
  });

  Future<AgeSignalState> requestWithResponse(Object? response) async {
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(ageSignalChannel, (call) async {
          expect(call.method, 'requestAgeSignal');
          expect(call.arguments, isNull);
          return response;
        });
    final container = ProviderContainer();
    addTearDown(container.dispose);

    await container.read(ageSignalProvider.notifier).request();
    return container.read(ageSignalProvider);
  }

  test('blocks when the signal upper bound is 17', () async {
    expect(
      await requestWithResponse({'status': 'signal', 'ageUpper': 17}),
      AgeSignalState.restricted,
    );
  });

  test('allows when the signal upper bound is 18', () async {
    expect(
      await requestWithResponse({'status': 'signal', 'ageUpper': 18}),
      AgeSignalState.allowed,
    );
  });

  test('allows when a signal has an open-ended upper bound', () async {
    expect(
      await requestWithResponse({'status': 'signal', 'ageUpper': null}),
      AgeSignalState.allowed,
    );
  });

  test('allows when no signal is available', () async {
    expect(
      await requestWithResponse({'status': 'noSignal', 'ageUpper': null}),
      AgeSignalState.allowed,
    );
  });

  test('exposes a gated retry after exhausted platform failures', () async {
    var requests = 0;
    var delays = 0;
    final provider = NotifierProvider<AgeSignalNotifier, AgeSignalState>(
      () => AgeSignalNotifier(
        requestSignal: () async {
          requests += 1;
          if (requests <= 2) {
            throw PlatformException(code: 'unavailable');
          }
          return {'status': 'noSignal', 'ageUpper': null};
        },
        delay: (duration) async {
          expect(duration, ageSignalRetryDelay);
          delays += 1;
        },
      ),
    );
    final container = ProviderContainer();
    addTearDown(container.dispose);

    await container.read(provider.notifier).request();

    expect(container.read(provider), AgeSignalState.retryableFailure);
    expect(requests, 2);
    expect(delays, 1);

    await container.read(provider.notifier).request();

    expect(container.read(provider), AgeSignalState.allowed);
    expect(requests, 3);
    expect(delays, 1);
  });

  test('retries a transient platform failure and applies the signal', () async {
    var requests = 0;
    final provider = NotifierProvider<AgeSignalNotifier, AgeSignalState>(
      () => AgeSignalNotifier(
        requestSignal: () async {
          requests += 1;
          if (requests == 1) {
            throw PlatformException(code: 'age_signal_unavailable');
          }
          return {'status': 'signal', 'ageUpper': 17};
        },
        delay: (duration) async {
          expect(duration, ageSignalRetryDelay);
        },
      ),
    );
    final container = ProviderContainer();
    addTearDown(container.dispose);

    await container.read(provider.notifier).request();

    expect(container.read(provider), AgeSignalState.restricted);
    expect(requests, 2);
  });

  test(
    'times out a stalled single native request and exposes a retry',
    () async {
      var requests = 0;
      var delays = 0;
      final provider = NotifierProvider<AgeSignalNotifier, AgeSignalState>(
        () => AgeSignalNotifier(
          requestSignal: () {
            requests += 1;
            return Completer<Map<Object?, Object?>?>().future;
          },
          delay: (duration) async {
            expect(duration, ageSignalRetryDelay);
            delays += 1;
          },
          requestTimeout: const Duration(milliseconds: 1),
          cancelSignal: () async => false,
        ),
      );
      final container = ProviderContainer();
      addTearDown(container.dispose);

      await container.read(provider.notifier).request();

      expect(container.read(provider), AgeSignalState.retryableFailure);
      expect(requests, 1);
      expect(delays, 1);
    },
  );

  test('a retry consumes the late result from a timed-out request', () async {
    var requests = 0;
    var delays = 0;
    final response = Completer<Map<Object?, Object?>?>();
    final provider = NotifierProvider<AgeSignalNotifier, AgeSignalState>(
      () => AgeSignalNotifier(
        requestSignal: () {
          requests += 1;
          return response.future;
        },
        delay: (duration) async {
          expect(duration, ageSignalRetryDelay);
          delays += 1;
          response.complete({'status': 'signal', 'ageUpper': 17});
        },
        requestTimeout: const Duration(milliseconds: 1),
        cancelSignal: () async => false,
      ),
    );
    final container = ProviderContainer();
    addTearDown(container.dispose);

    await container.read(provider.notifier).request();

    expect(container.read(provider), AgeSignalState.restricted);
    expect(requests, 1);
    expect(delays, 1);
  });

  test('a deliberate retry replaces an exhausted stalled request', () async {
    var requests = 0;
    var cancellations = 0;
    final provider = NotifierProvider<AgeSignalNotifier, AgeSignalState>(
      () => AgeSignalNotifier(
        requestSignal: () {
          requests += 1;
          if (requests == 1) {
            return Completer<Map<Object?, Object?>?>().future;
          }
          return Future.value({'status': 'noSignal', 'ageUpper': null});
        },
        delay: (_) async {},
        cancelSignal: () async {
          cancellations += 1;
          return true;
        },
        requestTimeout: const Duration(milliseconds: 1),
      ),
    );
    final container = ProviderContainer();
    addTearDown(container.dispose);

    await container.read(provider.notifier).request();
    expect(container.read(provider), AgeSignalState.retryableFailure);
    await container.read(provider.notifier).request();

    expect(container.read(provider), AgeSignalState.allowed);
    expect(requests, 2);
    expect(cancellations, 1);
  });

  test('an uncancellable stalled request remains the single flight', () async {
    var requests = 0;
    var restarts = 0;
    final provider = NotifierProvider<AgeSignalNotifier, AgeSignalState>(
      () => AgeSignalNotifier(
        requestSignal: () {
          requests += 1;
          return Completer<Map<Object?, Object?>?>().future;
        },
        delay: (_) async {},
        cancelSignal: () async => false,
        restartSignal: () async {
          restarts += 1;
        },
        requestTimeout: const Duration(milliseconds: 1),
      ),
    );
    final container = ProviderContainer();
    addTearDown(container.dispose);

    await container.read(provider.notifier).request();
    await container.read(provider.notifier).request();

    expect(container.read(provider), AgeSignalState.retryableFailure);
    expect(requests, 1);
    expect(restarts, 1);
  });

  test('a stalled cancellation still exposes the retry action', () async {
    var cancellations = 0;
    var restarts = 0;
    final provider = NotifierProvider<AgeSignalNotifier, AgeSignalState>(
      () => AgeSignalNotifier(
        requestSignal: () => Completer<Map<Object?, Object?>?>().future,
        delay: (_) async {},
        cancelSignal: () {
          cancellations += 1;
          return Completer<bool>().future;
        },
        restartSignal: () async {
          restarts += 1;
        },
        requestTimeout: const Duration(milliseconds: 1),
        cancellationTimeout: const Duration(milliseconds: 1),
      ),
    );
    final container = ProviderContainer();
    addTearDown(container.dispose);

    await container.read(provider.notifier).request();
    expect(container.read(provider), AgeSignalState.retryableFailure);
    await container.read(provider.notifier).request();

    expect(cancellations, 1);
    expect(restarts, 1);
  });

  test('a malformed cancellation still exposes the retry action', () async {
    final provider = NotifierProvider<AgeSignalNotifier, AgeSignalState>(
      () => AgeSignalNotifier(
        requestSignal: () => Completer<Map<Object?, Object?>?>().future,
        delay: (_) async {},
        cancelSignal: () async {
          final dynamic malformed = 'not-a-boolean';
          return malformed;
        },
        requestTimeout: const Duration(milliseconds: 1),
      ),
    );
    final container = ProviderContainer();
    addTearDown(container.dispose);

    await container.read(provider.notifier).request();

    expect(container.read(provider), AgeSignalState.retryableFailure);
  });

  test('keeps a missing native channel gated and retryable', () async {
    var requests = 0;
    final provider = NotifierProvider<AgeSignalNotifier, AgeSignalState>(
      () => AgeSignalNotifier(
        requestSignal: () async {
          requests += 1;
          throw MissingPluginException('buzz/age_signal');
        },
        delay: (duration) async {
          expect(duration, ageSignalRetryDelay);
        },
      ),
    );
    final container = ProviderContainer();
    addTearDown(container.dispose);

    await container.read(provider.notifier).request();

    expect(container.read(provider), AgeSignalState.retryableFailure);
    expect(requests, 2);
  });

  test('keeps malformed platform responses gated and retryable', () async {
    expect(
      await requestWithResponse({'status': 'unknown', 'ageUpper': null}),
      AgeSignalState.retryableFailure,
    );
    expect(
      await requestWithResponse({'status': 'signal', 'ageUpper': '17'}),
      AgeSignalState.retryableFailure,
    );
    expect(
      await requestWithResponse({
        'status': 'signal',
        'ageUpper': 17,
        'ageLower': 13,
      }),
      AgeSignalState.retryableFailure,
    );
    expect(
      await requestWithResponse({'status': 'noSignal', 'ageUpper': 17}),
      AgeSignalState.retryableFailure,
    );
    expect(await requestWithResponse(null), AgeSignalState.retryableFailure);
    expect(
      await requestWithResponse(['not', 'a', 'map']),
      AgeSignalState.retryableFailure,
    );
  });

  test('a deliberate retry can recover from a malformed response', () async {
    var requests = 0;
    final provider = NotifierProvider<AgeSignalNotifier, AgeSignalState>(
      () => AgeSignalNotifier(
        requestSignal: () async {
          requests += 1;
          return requests == 1
              ? {'status': 'unknown', 'ageUpper': null}
              : {'status': 'noSignal', 'ageUpper': null};
        },
      ),
    );
    final container = ProviderContainer();
    addTearDown(container.dispose);

    await container.read(provider.notifier).request();
    expect(container.read(provider), AgeSignalState.retryableFailure);

    await container.read(provider.notifier).request();
    expect(container.read(provider), AgeSignalState.allowed);
    expect(requests, 2);
  });

  test('a deliberate retry can recover from a null response', () async {
    var requests = 0;
    final provider = NotifierProvider<AgeSignalNotifier, AgeSignalState>(
      () => AgeSignalNotifier(
        requestSignal: () async {
          requests += 1;
          return requests == 1
              ? null
              : {'status': 'noSignal', 'ageUpper': null};
        },
      ),
    );
    final container = ProviderContainer();
    addTearDown(container.dispose);

    await container.read(provider.notifier).request();
    expect(container.read(provider), AgeSignalState.retryableFailure);

    await container.read(provider.notifier).request();
    expect(container.read(provider), AgeSignalState.allowed);
    expect(requests, 2);
  });

  test('requests the signal at most once', () async {
    var requests = 0;
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(ageSignalChannel, (_) async {
          requests += 1;
          return {'status': 'noSignal', 'ageUpper': null};
        });
    final container = ProviderContainer();
    addTearDown(container.dispose);
    final notifier = container.read(ageSignalProvider.notifier);

    await notifier.request();
    await notifier.request();

    expect(requests, 1);
  });

  test('remains checking until the platform request completes', () async {
    final response = Completer<Map<Object?, Object?>?>();
    final provider = NotifierProvider<AgeSignalNotifier, AgeSignalState>(
      () => AgeSignalNotifier(requestSignal: () => response.future),
    );
    final container = ProviderContainer();
    addTearDown(container.dispose);

    final request = container.read(provider.notifier).request();

    expect(container.read(provider), AgeSignalState.checking);
    response.complete({'status': 'noSignal', 'ageUpper': null});
    await request;
    expect(container.read(provider), AgeSignalState.allowed);
  });
}
