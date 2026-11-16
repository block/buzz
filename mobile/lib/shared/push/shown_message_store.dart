import 'dart:io';

import 'package:crypto/crypto.dart';

/// How far before the newest shown message a message may be and still be
/// shown. Wakes can be handled out of order, and the relay accepts a
/// `created_at` up to 15 minutes off, so a new message can carry an earlier
/// time than one already shown. Anything older is history. This is the same
/// window the iOS notification cache uses (`presentationReorderWindow`).
const shownMessageReorderWindow = Duration(minutes: 15);

/// The record of messages already shown in a notification, or seen in the
/// app, so later wakes and the app don't notify for them again.
///
/// The background wake handler and the foreground app both write it, from
/// different isolates and possibly different processes, so [claim] must be
/// atomic: of any number of concurrent claims for one message, exactly one
/// returns true.
abstract interface class ShownMessageStore {
  /// The newest time among the messages recorded for [community], or null
  /// when none is. Fetches start [shownMessageReorderWindow] before it.
  Future<DateTime?> newestShown(String community);

  /// Records [eventId] as shown and returns true, unless it is already
  /// recorded or is older than [shownMessageReorderWindow] before the newest
  /// recorded message, in which case it returns false and the caller must not
  /// notify.
  ///
  /// Callers claim before they post, so a message is posted at most once. If
  /// posting then fails, the message is not offered again, which is better
  /// than repeating it.
  Future<bool> claim(String community, String eventId, DateTime createdAt);
}

/// A [ShownMessageStore] kept as one empty marker file per message.
///
/// Creating a file exclusively is atomic across isolates and processes, with
/// no lock to take, so [claim] needs nothing else. The marker's name carries
/// the message's time, so [newestShown] needs no second file whose concurrent
/// read-modify-write could move it backwards.
///
/// Markers older than the reorder window before the newest one can never be
/// claimed again, so they are deleted.
class DirectoryShownMessageStore implements ShownMessageStore {
  DirectoryShownMessageStore(this._root, {DateTime Function()? now})
    : _now = now ?? DateTime.now;

  final Directory _root;
  final DateTime Function() _now;

  Directory _directory(String community) => Directory(
    '${_root.path}/${sha256.convert(community.codeUnits).toString()}',
  );

  /// The time a marker holds is capped at the present, so a future-dated
  /// message can't hold back later ones.
  int _watermark(DateTime createdAt) {
    final now = _now().millisecondsSinceEpoch ~/ 1000;
    final seconds = createdAt.millisecondsSinceEpoch ~/ 1000;
    return seconds < now ? seconds : now;
  }

  static final _marker = RegExp(r'^([0-9]{12})-([0-9a-f]{64})$');

  Future<List<({int watermark, File file})>> _markers(
    Directory directory,
  ) async {
    if (!await directory.exists()) return const [];
    return [
      await for (final entry in directory.list())
        if (entry is File)
          if (_marker.firstMatch(entry.uri.pathSegments.last) case final match?)
            (watermark: int.parse(match.group(1)!), file: entry),
    ];
  }

  @override
  Future<DateTime?> newestShown(String community) async {
    final markers = await _markers(_directory(community));
    if (markers.isEmpty) return null;
    final newest = markers.map((m) => m.watermark).reduce(_max);
    return DateTime.fromMillisecondsSinceEpoch(newest * 1000, isUtc: true);
  }

  @override
  Future<bool> claim(
    String community,
    String eventId,
    DateTime createdAt,
  ) async {
    if (!RegExp(r'^[0-9a-f]{64}$').hasMatch(eventId)) {
      throw ArgumentError.value(eventId, 'eventId', 'must be 64 hex digits');
    }
    final directory = _directory(community);
    await directory.create(recursive: true);
    final watermark = _watermark(createdAt);
    // An existing marker for this message may carry another watermark if the
    // clock moved, so look for it by id.
    final markers = await _markers(directory);
    if (markers.any((m) => m.file.uri.pathSegments.last.endsWith(eventId))) {
      return false;
    }
    final file = File(
      '${directory.path}/${watermark.toString().padLeft(12, '0')}-$eventId',
    );
    try {
      await file.create(exclusive: true);
    } on PathExistsException {
      return false;
    }
    // Check the window only after the marker exists. A concurrent claim that
    // advances the newest time may delete markers below its cutoff, and the
    // newest time never moves backwards because it is the maximum of marker
    // names, so a message that was shown and then pruned is rejected here
    // instead of being claimed a second time.
    final all = await _markers(directory);
    final newest = all.map((m) => m.watermark).reduce(_max);
    final cutoff = newest - shownMessageReorderWindow.inSeconds;
    for (final marker in all) {
      if (marker.watermark < cutoff) {
        try {
          await marker.file.delete();
        } on PathNotFoundException {
          // Another claim pruned it first.
        }
      }
    }
    return watermark >= cutoff;
  }
}

int _max(int a, int b) => a > b ? a : b;
