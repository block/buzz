import 'package:uuid/uuid.dart';

/// The time and tags for a message the user wrote in this app.
///
/// Matching Buzz desktop's outbox, a message carries a fresh `client-id` UUID
/// and the `ms` offset that orders same-second messages
/// (see `channelEventMilliseconds`). Pass [createdAt] as the event's
/// `created_at` so the two agree.
class ClientStamp {
  ClientStamp._(this.createdAt, this.tags);

  factory ClientStamp.now([DateTime? at]) {
    final ms = (at ?? DateTime.now()).millisecondsSinceEpoch;
    return ClientStamp._(ms ~/ 1000, [
      ['client-id', const Uuid().v4()],
      ['ms', '${ms % 1000}'],
    ]);
  }

  /// Unix seconds.
  final int createdAt;

  final List<List<String>> tags;
}
