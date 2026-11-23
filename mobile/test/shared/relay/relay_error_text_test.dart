import 'dart:async';

import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  for (final (error, text) in <(Object, String)>[
    (Exception('Upload failed'), 'Upload failed'),
    (
      TimeoutException('Event abc not acknowledged within 0:00:10'),
      'TimeoutException: Event abc not acknowledged within 0:00:10',
    ),
    (StateError('Exception: kept'), 'Bad state: Exception: kept'),
    (
      const RelayEventRejectedException('restricted: guests'),
      'restricted: guests',
    ),
    (
      RelayAckLostError(Exception('App moved to background')),
      'Connection lost before the relay acknowledged the event: '
          'App moved to background',
    ),
  ]) {
    test('shows ${error.runtimeType} as "$text"', () {
      expect(relayErrorText(error), text);
    });
  }
}
