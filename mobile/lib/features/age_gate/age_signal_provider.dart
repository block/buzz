import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

const ageSignalChannel = MethodChannel('buzz/age_signal');

/// Maximum lifetime of a launch age check. Access never waits for this timer.
const ageSignalRequestTimeout = Duration(seconds: 30);

/// Invokes the native age-signal request.
typedef AgeSignalRequest = Future<Map<Object?, Object?>?> Function();

Future<Map<Object?, Object?>?> _requestPlatformAgeSignal() =>
    ageSignalChannel.invokeMapMethod<Object?, Object?>('requestAgeSignal');

/// Returns true only for an explicit, valid, inclusive upper age below 18.
/// Unknown envelopes and impossible bounds cannot authorize restriction.
bool shouldBlockForAgeSignal(Map<Object?, Object?> response) {
  if (response.length != 2 ||
      response['status'] != 'signal' ||
      !response.containsKey('ageUpper')) {
    return false;
  }
  final upper = response['ageUpper'];
  return upper is int && upper >= 0 && upper < 18;
}

/// Access decision, independent of whether a platform check has completed.
enum AgeSignalState { allowed, restricted }

/// Observes one native result per launch without gating normal startup.
class AgeSignalNotifier extends Notifier<AgeSignalState> {
  AgeSignalNotifier({
    AgeSignalRequest? requestSignal,
    Duration requestTimeout = ageSignalRequestTimeout,
  }) : _requestSignal = requestSignal ?? _requestPlatformAgeSignal,
       _requestTimeout = requestTimeout;

  final AgeSignalRequest _requestSignal;
  final Duration _requestTimeout;
  Future<void>? _attempt;
  int _generation = 0;

  @override
  AgeSignalState build() {
    // Invalidation/disposal retires the request even if native code completes
    // later. A new provider lifecycle starts allowed, without cached denial.
    _generation += 1;
    _attempt = null;
    ref.onDispose(() => _generation += 1);
    return AgeSignalState.allowed;
  }

  /// Checks once. Failure ends the attempt; neither retry nor cleanup gates UI.
  Future<void> request() => _attempt ??= _check(++_generation);

  Future<void> _check(int generation) async {
    try {
      final response = await Future.sync(
        _requestSignal,
      ).timeout(_requestTimeout);
      if (generation != _generation) return;
      // Retire before interpreting the result. Only this current completion
      // can commit a restriction; timeout and abandoned callbacks cannot.
      _generation += 1;
      if (response != null && shouldBlockForAgeSignal(response)) {
        state = AgeSignalState.restricted;
      }
    } catch (error) {
      if (generation != _generation) return;
      _generation += 1;
      // This is an intentional fail-open boundary, including codec and plugin
      // errors. Record only the category, never age data or exception payloads.
      debugPrint(
        'Age check unavailable (${error.runtimeType}); access allowed.',
      );
    }
  }
}

/// Explicit dogfood opt-in. Production builds keep enforcement disabled.
const ageGatingEnabled = bool.fromEnvironment('BUZZ_AGE_GATING_ENABLED');

final ageSignalProvider = NotifierProvider<AgeSignalNotifier, AgeSignalState>(
  ageGatingEnabled ? AgeSignalNotifier.new : _DisabledAgeSignalNotifier.new,
);

// Remains disabled until the fail-open implementation is validated in dogfood.
class _DisabledAgeSignalNotifier extends AgeSignalNotifier {
  @override
  Future<void> request() async {}
}
