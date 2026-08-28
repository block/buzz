part of 'channels_provider.dart';

// This extension exists only to keep `channels_provider.dart` under the
// desktop/mobile file-size ratchet; `_clearLiveSubscriptions` is not a
// standalone semantic boundary and belongs to `ChannelsNotifier`.
extension _ChannelsNotifierSubscriptionCleanup on ChannelsNotifier {
  void _clearLiveSubscriptions() {
    _subscriptionVersion++;
    _desiredLiveChannelIds = const {};
    for (final unsubscribe in _unsubscribersByChannel.values) {
      unsubscribe();
    }
    _unsubscribersByChannel.clear();
    _subscriptionRelayBaseUrl = null;
    _backstopTimer?.cancel();
    _backstopTimer = null;
  }
}
