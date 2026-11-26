import 'package:buzz/features/channels/create_channel_error.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('a failed channel create is described without internal error text', () {
    final cases = <Object, String>{
      const RelayEventRejectedException(
        'invalid: an open channel named "general" already exists',
      ): 'An open channel named "general" already exists',
      RelayNotConnectedError(): 'Couldn’t create channel. No connection.',
      RelayAckTimeoutError(
        'Event 371d27aa not acknowledged within 0:00:08.000000',
        const Duration(seconds: 8),
      ): "Not confirmed. The relay didn't respond.",
      RelayAckLostError(StateError('socket closed')):
          'Not confirmed. Connection lost.',
      // What createChannel throws when the community changes mid-create.
      ActiveCommunityChangedError(
        'Channel action cancelled because the active community changed',
      ): 'Channel action cancelled because the active community changed',
      // Anything else keeps its own text, minus the relay's machine prefix.
      Exception('restricted: not allowed to create channels'):
          'Not allowed to create channels',
    };
    for (final MapEntry(key: error, value: message) in cases.entries) {
      expect(createChannelErrorMessage(error), message, reason: '$error');
    }
  });
}
