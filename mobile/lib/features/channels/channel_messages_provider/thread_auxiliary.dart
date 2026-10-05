part of '../channel_messages_provider.dart';

extension _ThreadAuxiliaryState on ChannelMessagesNotifier {
  void _cacheThreadAuxiliaryEvents(
    String rootId,
    List<NostrEvent> replies,
    List<NostrEvent> auxiliaryEvents,
  ) {
    if (auxiliaryEvents.isEmpty) return;
    final scopedIds = {
      rootId,
      ...replies.map((event) => event.id),
      ...auxiliaryEvents.map((event) => event.id),
    };
    final deletions = <NostrEvent>[];
    var events = _lastKnownMessages ?? const <NostrEvent>[];
    for (final event in auxiliaryEvents) {
      if (event.kind == EventKind.deletion ||
          event.kind == EventKind.nip29DeleteEvent) {
        deletions.add(event);
        continue;
      }
      if (event.channelId != channelId ||
          !EventKind.channelAuxEventKinds.contains(event.kind)) {
        throw StateError('Expected an auxiliary event in channel $channelId.');
      }
      // Do not use the live-event path: it invalidates the query that just
      // fetched this overlay. Only content replies confirm local sends.
      _mergeWindowEventIntoStore(event);
      if (!_usingChannelWindow) {
        events = ChannelMessagesNotifier._mergeEvent(events, event);
      }
    }
    if (_usingChannelWindow) {
      events = _withDeepLinkEvents(flattenChannelWindowEvents(_windowStore));
    }
    _lastKnownMessages = events;
    _publishSummaryChange();
    cacheThreadDeletions(deletions, scopedTargetIds: scopedIds);
  }
}
