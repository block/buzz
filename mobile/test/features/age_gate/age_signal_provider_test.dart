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

  Future<bool> requestWithResponse(Object? response) async {
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
      isTrue,
    );
  });

  test('allows when the signal upper bound is 18', () async {
    expect(
      await requestWithResponse({'status': 'signal', 'ageUpper': 18}),
      isFalse,
    );
  });

  test('allows when a signal has an open-ended upper bound', () async {
    expect(
      await requestWithResponse({'status': 'signal', 'ageUpper': null}),
      isFalse,
    );
  });

  test('allows when no signal is available', () async {
    expect(
      await requestWithResponse({'status': 'noSignal', 'ageUpper': null}),
      isFalse,
    );
  });

  test('allows when the platform request throws', () async {
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(ageSignalChannel, (_) async {
          throw PlatformException(code: 'unavailable');
        });
    final container = ProviderContainer();
    addTearDown(container.dispose);

    await container.read(ageSignalProvider.notifier).request();

    expect(container.read(ageSignalProvider), isFalse);
  });

  test('allows malformed and unexpected platform responses', () async {
    expect(
      await requestWithResponse({'status': 'unknown', 'ageUpper': null}),
      isFalse,
    );
    expect(
      await requestWithResponse({'status': 'signal', 'ageUpper': '17'}),
      isFalse,
    );
    expect(
      await requestWithResponse({
        'status': 'signal',
        'ageUpper': 17,
        'ageLower': 13,
      }),
      isFalse,
    );
    expect(await requestWithResponse(null), isFalse);
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
}
