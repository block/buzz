import 'package:flutter/foundation.dart';
import 'package:intl/intl.dart';

// Re-export shortPubkey so existing callers continue to compile.
export '../../shared/utils/string_utils.dart' show shortPubkey;

final _fullDateFormat = DateFormat('EEEE, MMMM d, y');
final _shortMonthFormat = DateFormat('MMM');
final _messageTimeFormat = DateFormat('h:mm a', 'en_US');

/// Returns "Today", "Yesterday", or a full date like "Monday, March 31, 2026".
///
/// [now] is exposed for testing; production callers should omit it.
String formatDayHeading(int unixSeconds, {@visibleForTesting DateTime? now}) {
  final date = DateTime.fromMillisecondsSinceEpoch(
    unixSeconds * 1000,
    isUtc: true,
  ).toLocal();
  now ??= DateTime.now();
  final today = DateTime(now.year, now.month, now.day);
  final messageDay = DateTime(date.year, date.month, date.day);

  if (today.year == messageDay.year &&
      today.month == messageDay.month &&
      today.day == messageDay.day) {
    return 'Today';
  }
  final yesterday = DateTime(now.year, now.month, now.day - 1);
  if (yesterday.year == messageDay.year &&
      yesterday.month == messageDay.month &&
      yesterday.day == messageDay.day) {
    return 'Yesterday';
  }
  return _fullDateFormat.format(date);
}

/// Whether two unix-second timestamps fall on the same calendar day (local time).
bool isSameDay(int a, int b) {
  final dtA = DateTime.fromMillisecondsSinceEpoch(
    a * 1000,
    isUtc: true,
  ).toLocal();
  final dtB = DateTime.fromMillisecondsSinceEpoch(
    b * 1000,
    isUtc: true,
  ).toLocal();
  return dtA.year == dtB.year && dtA.month == dtB.month && dtA.day == dtB.day;
}

/// Returns a compact relative time string like "just now", "5m ago", "3h ago",
/// "2d ago", or a short date for older timestamps.
String relativeTime(int unixSeconds) {
  final now = DateTime.now();
  final time = DateTime.fromMillisecondsSinceEpoch(
    unixSeconds * 1000,
    isUtc: true,
  ).toLocal();
  final diff = now.difference(time);

  if (diff.inMinutes < 1) return 'just now';
  if (diff.inMinutes < 60) return '${diff.inMinutes}m ago';
  if (diff.inHours < 24) return '${diff.inHours}h ago';
  if (diff.inDays < 7) return '${diff.inDays}d ago';
  return '${time.month}/${time.day}/${time.year}';
}

/// Returns desktop-parity thread activity copy such as "just now",
/// "3 hours ago", or "on May 19th".
String formatThreadSummaryLastReplyTime(
  int unixSeconds, {
  @visibleForTesting int? nowSeconds,
}) {
  nowSeconds ??= DateTime.now().millisecondsSinceEpoch ~/ 1000;
  var diff = nowSeconds - unixSeconds;
  if (diff < 0) diff = 0;

  if (diff < 60) return 'just now';
  if (diff < 3600) return _formatAgo(diff ~/ 60, 'minute');
  if (diff < 86400) return _formatAgo(diff ~/ 3600, 'hour');
  if (diff < 604800) return _formatAgo(diff ~/ 86400, 'day');

  final date = DateTime.fromMillisecondsSinceEpoch(
    unixSeconds * 1000,
    isUtc: true,
  ).toLocal();
  return 'on ${_shortMonthFormat.format(date)} '
      '${date.day}${_ordinalSuffix(date.day)}';
}

String _formatAgo(int value, String unit) =>
    '$value $unit${value == 1 ? '' : 's'} ago';

String _ordinalSuffix(int day) {
  final lastTwoDigits = day % 100;
  if (lastTwoDigits >= 11 && lastTwoDigits <= 13) return 'th';
  return switch (day % 10) {
    1 => 'st',
    2 => 'nd',
    3 => 'rd',
    _ => 'th',
  };
}

/// Desktop-parity message clock time, e.g. "2:34 PM".
String formatMessageTime(int unixSeconds) {
  final date = DateTime.fromMillisecondsSinceEpoch(
    unixSeconds * 1000,
    isUtc: true,
  ).toLocal();
  return _messageTimeFormat.format(date);
}
