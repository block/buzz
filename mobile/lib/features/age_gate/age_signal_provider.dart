import 'package:flutter/services.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

const ageSignalChannel = MethodChannel('buzz/age_signal');

bool shouldBlockForAgeSignal(Map<Object?, Object?> response) {
  if (response.length != 2 ||
      !response.containsKey('status') ||
      !response.containsKey('ageUpper')) {
    throw StateError('Unexpected age signal response.');
  }

  final status = response['status'];
  if (status == 'noSignal') {
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

class AgeSignalNotifier extends Notifier<bool> {
  bool _requested = false;

  @override
  bool build() => false;

  Future<void> request() async {
    if (_requested) {
      return;
    }
    _requested = true;

    try {
      final response = await ageSignalChannel.invokeMapMethod<Object?, Object?>(
        'requestAgeSignal',
      );
      if (response != null) {
        state = shouldBlockForAgeSignal(response);
      }
    } on Object {
      state = false;
    }
  }
}

final ageSignalProvider = NotifierProvider<AgeSignalNotifier, bool>(
  AgeSignalNotifier.new,
);
