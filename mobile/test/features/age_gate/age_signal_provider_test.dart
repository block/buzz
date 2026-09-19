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

  ProviderContainer enabledContainer({AgeSignalNotifier Function()? create}) {
    final container = ProviderContainer(
      overrides: [
        ageSignalProvider.overrideWith(create ?? AgeSignalNotifier.new),
      ],
    );
    addTearDown(container.dispose);
    return container;
  }

  test('production provider respects the explicit dogfood opt-in', () async {
    var calls = 0;
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(ageSignalChannel, (_) async {
          calls += 1;
          return {'status': 'signal', 'ageUpper': 17};
        });
    final container = ProviderContainer();
    addTearDown(container.dispose);
    await container.read(ageSignalProvider.notifier).request();
    expect(
      container.read(ageSignalProvider),
      ageGatingEnabled ? AgeSignalState.restricted : AgeSignalState.allowed,
    );
    expect(calls, ageGatingEnabled ? 1 : 0);
  });

  for (final upper in [-100, -1, 0, 1, 12, 16, 17, 18, 19, 120, 999999]) {
    test('native inclusive upper bound $upper', () async {
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(
            ageSignalChannel,
            (_) async => {'status': 'signal', 'ageUpper': upper},
          );
      final container = enabledContainer();
      expect(container.read(ageSignalProvider), AgeSignalState.allowed);
      await container.read(ageSignalProvider.notifier).request();
      expect(
        container.read(ageSignalProvider),
        upper >= 0 && upper < 18
            ? AgeSignalState.restricted
            : AgeSignalState.allowed,
      );
    });
  }

  final invalid = <Object?>[
    null,
    'wrong envelope',
    ['signal', 17],
    {},
    {'status': 'signal'},
    {'ageUpper': 17},
    {'status': 'unknown', 'ageUpper': 17},
    {'status': 'noSignal', 'ageUpper': 17},
    {'status': 'signal', 'ageUpper': '17'},
    {'status': 'signal', 'ageUpper': 17.0},
    {'status': 'signal', 'ageUpper': true},
    {'status': 'signal', 'ageUpper': null},
    {'status': 'signal', 'ageUpper': 17, 'unexpected': true},
  ];
  for (var index = 0; index < invalid.length; index++) {
    test('malformed or unknown native response $index allows access', () async {
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(
            ageSignalChannel,
            (_) async => invalid[index],
          );
      final container = enabledContainer();
      await container.read(ageSignalProvider.notifier).request();
      expect(container.read(ageSignalProvider), AgeSignalState.allowed);
    });
  }

  for (final error in <Object>[
    PlatformException(code: 'unavailable'),
    MissingPluginException(),
    StateError('synchronous failure'),
    FormatException('bad codec'),
    ArgumentError('unexpected integration error'),
  ]) {
    test('${error.runtimeType} preserves access and is not retried', () async {
      var calls = 0;
      final container = enabledContainer(
        create: () => AgeSignalNotifier(
          requestSignal: () {
            calls++;
            throw error;
          },
        ),
      );
      final notifier = container.read(ageSignalProvider.notifier);
      await notifier.request();
      await notifier.request();
      expect(container.read(ageSignalProvider), AgeSignalState.allowed);
      expect(calls, 1);
    });
  }

  test('platform exception crosses the real channel and fails open', () async {
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(ageSignalChannel, (_) async {
          throw PlatformException(
            code: 'age_signal_notification_protection_failed',
          );
        });
    final container = enabledContainer();
    await container.read(ageSignalProvider.notifier).request();
    expect(container.read(ageSignalProvider), AgeSignalState.allowed);
  });

  test('missing native channel fails open', () async {
    final container = enabledContainer();
    await container.read(ageSignalProvider.notifier).request();
    expect(container.read(ageSignalProvider), AgeSignalState.allowed);
  });

  test(
    'pending request allows access and concurrent callers share it',
    () async {
      final response = Completer<Map<Object?, Object?>?>();
      var calls = 0;
      final container = enabledContainer(
        create: () => AgeSignalNotifier(
          requestSignal: () {
            calls++;
            return response.future;
          },
        ),
      );
      final notifier = container.read(ageSignalProvider.notifier);
      final first = notifier.request();
      final second = notifier.request();
      expect(container.read(ageSignalProvider), AgeSignalState.allowed);
      response.complete({'status': 'signal', 'ageUpper': 17});
      await Future.wait([first, second]);
      expect(calls, 1);
      expect(container.read(ageSignalProvider), AgeSignalState.restricted);
    },
  );

  test('timeout retires request even when late result is under 18', () async {
    final response = Completer<Map<Object?, Object?>?>();
    final container = enabledContainer(
      create: () => AgeSignalNotifier(
        requestSignal: () => response.future,
        requestTimeout: Duration.zero,
      ),
    );
    final notifier = container.read(ageSignalProvider.notifier);
    await notifier.request();
    expect(container.read(ageSignalProvider), AgeSignalState.allowed);
    response.complete({'status': 'signal', 'ageUpper': 17});
    await response.future;
    await notifier.request();
    expect(container.read(ageSignalProvider), AgeSignalState.allowed);
  });

  test('invalidating a provider retires the previous request', () async {
    final response = Completer<Map<Object?, Object?>?>();
    final container = enabledContainer(
      create: () => AgeSignalNotifier(requestSignal: () => response.future),
    );
    final request = container.read(ageSignalProvider.notifier).request();
    container.invalidate(ageSignalProvider);
    expect(container.read(ageSignalProvider), AgeSignalState.allowed);
    response.complete({'status': 'signal', 'ageUpper': 17});
    await request;
    expect(container.read(ageSignalProvider), AgeSignalState.allowed);
  });

  test('disposal retires the request without an unhandled error', () async {
    final response = Completer<Map<Object?, Object?>?>();
    final container = ProviderContainer(
      overrides: [
        ageSignalProvider.overrideWith(
          () => AgeSignalNotifier(requestSignal: () => response.future),
        ),
      ],
    );
    final request = container.read(ageSignalProvider.notifier).request();
    container.dispose();
    response.complete({'status': 'signal', 'ageUpper': 17});
    await request;
  });
}
