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

/// A [ShownMessageStore] kept as two empty marker files per message.
///
/// The claim marker's name holds only the message's own time and id, which
/// never change, so every claim for one message names the same file, and
/// creating it exclusively is atomic across isolates and processes with no
/// lock to take. The claim that creates it wins.
///
/// The winner then writes a watermark marker: the message's time capped at the
/// present, so a future-dated message can't hold back later ones.
/// [newestShown] is the newest watermark, so it needs no shared cursor file
/// whose concurrent read-modify-write could move it backwards. If the process
/// dies between the two files, the fetch only starts earlier, and the claim
/// markers drop what was already shown.
///
/// Markers whose message is older than the reorder window before the newest
/// watermark can never be claimed again, so they are deleted. Retention
/// follows the message's own time, not the capped watermark: a future-dated
/// message still matches the fetch window after its watermark has aged out, so
/// its claim marker must outlive that.
class DirectoryShownMessageStore implements ShownMessageStore {
  DirectoryShownMessageStore(this._root, {DateTime Function()? now})
    : _now = now ?? DateTime.now;

  final Directory _root;
  final DateTime Function() _now;

  Directory _directory(String community) => Directory(
    '${_root.path}/${sha256.convert(community.codeUnits).toString()}',
  );

  static const _maxMarkerSeconds = 999999999999;

  static int _seconds(DateTime time) =>
      (time.millisecondsSinceEpoch ~/ 1000).clamp(0, _maxMarkerSeconds);

  static String _digits(int seconds) => seconds.toString().padLeft(12, '0');

  static final _marker = RegExp(r'^([cw])-([0-9]{12})-([0-9a-f]{64})$');

  Future<List<({bool claim, int seconds, File file})>> _markers(
    Directory directory,
  ) async {
    if (!await directory.exists()) return const [];
    return [
      await for (final entry in directory.list())
        if (entry is File)
          if (_marker.firstMatch(entry.uri.pathSegments.last) case final match?)
            (
              claim: match.group(1) == 'c',
              seconds: int.parse(match.group(2)!),
              file: entry,
            ),
    ];
  }

  static int? _newest(Iterable<({bool claim, int seconds, File file})> all) {
    final watermarks = [
      for (final marker in all)
        if (!marker.claim) marker.seconds,
    ];
    return watermarks.isEmpty ? null : watermarks.reduce(_max);
  }

  @override
  Future<DateTime?> newestShown(String community) async {
    final newest = _newest(await _markers(_directory(community)));
    if (newest == null) return null;
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
    final created = _seconds(createdAt);
    try {
      await File(
        '${directory.path}/c-${_digits(created)}-$eventId',
      ).create(exclusive: true);
    } on PathExistsException {
      return false;
    }
    // The claim is won and can't be undone, so nothing after this may throw:
    // a throw would leave a claim that no later wake can win, for a message
    // never shown. If the bookkeeping fails, the message is shown. A missing
    // watermark only makes later fetches start earlier, and a skipped prune
    // runs again on the next claim.
    try {
      final now = _seconds(_now());
      final watermark = created < now ? created : now;
      await File('${directory.path}/w-${_digits(watermark)}-$eventId').create();
      // Check the window only after the markers exist. A concurrent claim that
      // advances the newest watermark may delete markers below its cutoff, and
      // the newest watermark never moves backwards because it is the maximum
      // of marker names and is never itself below a cutoff, so a message that
      // was shown and then pruned is rejected here instead of being claimed
      // again.
      final all = await _markers(directory);
      final cutoff = _newest(all)! - shownMessageReorderWindow.inSeconds;
      for (final marker in all) {
        if (marker.seconds < cutoff) {
          try {
            await marker.file.delete();
          } on PathNotFoundException {
            // Another claim pruned it first.
          }
        }
      }
      return created >= cutoff;
    } on Object {
      return true;
    }
  }
}

int _max(int a, int b) => a > b ? a : b;
