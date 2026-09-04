import 'dart:async';

import 'package:flutter/services.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

const ageSignalChannel = MethodChannel('buzz/age_signal');

/// Delay before the single retry of a failed native age-signal request.
const ageSignalRetryDelay = Duration(seconds: 1);

/// Maximum time allowed for each native age-signal request attempt.
const ageSignalRequestTimeout = Duration(seconds: 30);

/// Maximum time allowed for native cancellation acknowledgement.
const ageSignalCancellationTimeout = Duration(seconds: 5);

/// Invokes the native age-signal request.
typedef AgeSignalRequest = Future<Map<Object?, Object?>?> Function();

/// Waits before retrying a failed native age-signal request.
typedef AgeSignalDelay = Future<void> Function(Duration duration);
typedef AgeSignalCancel = Future<bool> Function();
typedef AgeSignalRestart = Future<void> Function();

Future<Map<Object?, Object?>?> _requestPlatformAgeSignal() =>
    ageSignalChannel.invokeMapMethod<Object?, Object?>('requestAgeSignal');

Future<void> _delayAgeSignalRetry(Duration duration) =>
    Future<void>.delayed(duration);
Future<bool> _cancelPlatformAgeSignal() async =>
    await ageSignalChannel.invokeMethod<bool>('cancelAgeSignalRequest') ??
    false;
Future<void> _restartForPlatformAgeSignal() =>
    ageSignalChannel.invokeMethod<void>('restartForAgeSignal');

bool shouldBlockForAgeSignal(Map<Object?, Object?> response) {
  if (response.length != 2 ||
      !response.containsKey('status') ||
      !response.containsKey('ageUpper')) {
    throw StateError('Unexpected age signal response.');
  }

  final status = response['status'];
  if (status == 'noSignal') {
    if (response['ageUpper'] != null) {
      throw StateError('Unexpected age signal upper bound.');
    }
    return false;
  }
  if (status != 'signal') {
    throw StateError('Unexpected age signal status.');
  }

  final ageUpper = response['ageUpper'];
  if (ageUpper == null) {
    return false;
  }
  if (ageUpper is! int) {
    throw StateError('Unexpected age signal upper bound.');
  }
  return ageUpper < 18;
}

/// Result of the platform age-signal check for this app launch.
enum AgeSignalState { checking, retryableFailure, allowed, restricted }

class AgeSignalNotifier extends Notifier<AgeSignalState> {
  /// Creates an age-signal notifier, optionally with test request hooks.
  AgeSignalNotifier({
    AgeSignalRequest? requestSignal,
    AgeSignalDelay? delay,
    AgeSignalCancel? cancelSignal,
    AgeSignalRestart? restartSignal,
    Duration requestTimeout = ageSignalRequestTimeout,
    Duration cancellationTimeout = ageSignalCancellationTimeout,
  }) : _requestSignal = requestSignal ?? _requestPlatformAgeSignal,
       _delay = delay ?? _delayAgeSignalRetry,
       _cancelSignal = cancelSignal ?? _cancelPlatformAgeSignal,
       _restartSignal = restartSignal ?? _restartForPlatformAgeSignal,
       _requestTimeout = requestTimeout,
       _cancellationTimeout = cancellationTimeout;

  static const _maxAttempts = 2;

  final AgeSignalRequest _requestSignal;
  final AgeSignalDelay _delay;
  final AgeSignalCancel _cancelSignal;
  final AgeSignalRestart _restartSignal;
  final Duration _requestTimeout;
  final Duration _cancellationTimeout;
  bool _completed = false;
  bool _restartRequired = false;
  Future<void>? _requestInFlight;
  Future<Map<Object?, Object?>?>? _nativeRequestInFlight;

  @override
  AgeSignalState build() => AgeSignalState.checking;

  Future<void> request() async {
    if (_completed) return;
    if (_restartRequired) {
      await _restartSignal();
      return;
    }

    final requestInFlight = _requestInFlight;
    if (requestInFlight != null) {
      await requestInFlight;
      return;
    }

    state = AgeSignalState.checking;
    final request = _requestWithRetry();
    _requestInFlight = request;
    try {
      await request;
    } finally {
      if (identical(_requestInFlight, request)) {
        _requestInFlight = null;
      }
    }
  }

  Future<void> _requestWithRetry() async {
    for (var attempt = 0; attempt < _maxAttempts; attempt += 1) {
      final Map<Object?, Object?>? response;
      try {
        response = await _awaitNativeRequest();
      } on MissingPluginException {
        if (attempt + 1 < _maxAttempts) {
          await _delay(ageSignalRetryDelay);
          continue;
        }
        // A missing channel is an integration failure, not evidence that the
        // platform has no age signal. Keep the launch gated for a retry.
        state = AgeSignalState.retryableFailure;
        return;
      } on PlatformException {
        if (attempt + 1 < _maxAttempts) {
          await _delay(ageSignalRetryDelay);
          continue;
        }
        // A transient native failure is not evidence that access is allowed.
        // Keep the launch gated and expose a deliberate retry action.
        state = AgeSignalState.retryableFailure;
        return;
      } on TimeoutException {
        if (attempt + 1 < _maxAttempts) {
          await _delay(ageSignalRetryDelay);
          continue;
        }
        await _retireNativeRequest();
        // A stalled platform flow is not evidence that access is allowed.
        // Keep the launch gated and expose a deliberate retry action.
        state = AgeSignalState.retryableFailure;
        return;
      } on TypeError {
        // A method-channel envelope with the wrong shape fails before the map
        // validator runs. Keep the launch gated with an explicit retry.
        state = AgeSignalState.retryableFailure;
        return;
      }

      final bool shouldBlock;
      try {
        if (response == null) {
          throw StateError('Missing age signal response.');
        }
        shouldBlock = shouldBlockForAgeSignal(response);
      } on StateError {
        // A malformed native response is an integration failure, not evidence
        // that access is allowed. Keep the launch gated for a deliberate retry.
        state = AgeSignalState.retryableFailure;
        return;
      }

      _completed = true;
      state = shouldBlock ? AgeSignalState.restricted : AgeSignalState.allowed;
      return;
    }
  }

  Future<Map<Object?, Object?>?> _awaitNativeRequest() async {
    final request = _nativeRequestInFlight ??= _requestSignal();
    try {
      final response = await request.timeout(_requestTimeout);
      if (identical(_nativeRequestInFlight, request)) {
        _nativeRequestInFlight = null;
      }
      return response;
    } on TimeoutException {
      // A Dart timeout cannot cancel the platform consent flow. Keep this
      // future as the single flight so retries cannot start overlapping native
      // prompts and can still consume a late response.
      rethrow;
    } catch (_) {
      if (identical(_nativeRequestInFlight, request)) {
        _nativeRequestInFlight = null;
      }
      rethrow;
    }
  }

  Future<void> _retireNativeRequest() async {
    var retired = false;
    try {
      retired = await _cancelSignal().timeout(_cancellationTimeout);
    } on TimeoutException {
      // Stay gated if cancellation does not acknowledge within its deadline.
    } on MissingPluginException {
      // Stay gated if cancellation cannot be acknowledged.
    } on PlatformException {
      // Stay gated if cancellation cannot be acknowledged.
    } on TypeError {
      // Stay gated if the method channel returns a malformed acknowledgement.
    } finally {
      if (retired) {
        _nativeRequestInFlight = null;
      } else {
        _restartRequired = true;
      }
    }
  }
}

final ageSignalProvider = NotifierProvider<AgeSignalNotifier, AgeSignalState>(
  AgeSignalNotifier.new,
);
