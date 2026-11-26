import '../../shared/relay/relay.dart';

/// Text for a failed channel create, for both creation sheets.
///
/// Failures after the EVENT went out say "Not confirmed": the relay may have
/// created the channel.
String createChannelErrorMessage(Object error) => switch (error) {
  RelayNotConnectedError() => 'Couldn’t create channel. No connection.',
  RelayAckTimeoutError() => "Not confirmed. The relay didn't respond.",
  RelayAckLostError() => 'Not confirmed. Connection lost.',
  // Any other StateError, such as a community switch during the create,
  // prints as `Bad state: ...`; its message is the sentence.
  StateError(:final message) => message,
  _ => relayErrorMessage(error),
};
