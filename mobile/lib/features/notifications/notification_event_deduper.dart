import 'dart:collection';

class NotificationEventDeduper {
  NotificationEventDeduper({this.capacity = 1000}) : assert(capacity > 0);

  final int capacity;
  final LinkedHashSet<String> _eventIds = LinkedHashSet<String>();

  /// Remembers [eventId], returning false when it was already seen.
  bool add(String eventId) {
    if (_eventIds.contains(eventId)) return false;
    if (_eventIds.length == capacity) {
      _eventIds.remove(_eventIds.first);
    }
    _eventIds.add(eventId);
    return true;
  }

  /// Forgets [eventId] after a failed delivery so a later relay copy can retry.
  void remove(String eventId) => _eventIds.remove(eventId);
}
