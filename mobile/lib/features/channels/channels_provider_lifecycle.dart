part of 'channels_provider.dart';

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
