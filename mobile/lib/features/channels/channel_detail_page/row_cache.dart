part of '../channel_detail_page.dart';

/// Built timeline rows, kept for reuse.
///
/// A rebuild of the message list (a new bottom inset as the composer grows
/// or the keyboard moves, or a page dependency settling after a reconnect)
/// builds new row widgets, and Flutter rebuilds every visible row for them.
/// Returning the previous widget instead skips that work. Anything a row reads
/// from providers still rebuilds it through its own watch.
_RowCache _useRowCache(_MessageList list, List<Object?> pageInputs) {
  final cache = useMemoized(_RowCache.new);
  cache.retainFor(list, pageInputs);
  return cache;
}

class _RowCache {
  static const _limit = 400;

  final Map<String, _BuiltRow> _rows = {};
  List<Object?> _shared = const [];

  /// Starts over when anything every row is built from has changed.
  void retainFor(_MessageList list, List<Object?> pageInputs) {
    final shared = [
      ...pageInputs,
      list.allMessages,
      list.channelId,
      list.currentPubkey,
      list.isMember,
      list.isArchived,
      list.composerFocusNode,
      list.restoreComposerFocus,
      list.welcomeAgent,
      list.notifier.reachedOldest,
      TypingMotionPreview.enabled,
    ];
    if (_sameInputs(_shared, shared)) return;
    _rows.clear();
    _shared = shared;
  }

  /// The row built for [message] from the same [inputs], if any.
  Widget? reuse(TimelineMessage message, List<Object?> inputs) {
    final row = _rows[message.id];
    if (row == null || !identical(row.message, message)) return null;
    return _sameInputs(row.inputs, inputs) ? row.widget : null;
  }

  Widget keep(TimelineMessage message, List<Object?> inputs, Widget widget) {
    // Scrolling builds rows without a list rebuild, so bound the cache here.
    if (_rows.length >= _limit) _rows.clear();
    _rows[message.id] = (message: message, inputs: inputs, widget: widget);
    return widget;
  }
}

typedef _BuiltRow = ({
  TimelineMessage message,
  List<Object?> inputs,
  Widget widget,
});

/// Everything one timeline row is built from beyond the shared inputs.
///
/// [rowInputs] holds the neighbors, which decide the day divider, author and
/// avatar, and anything else particular to the row.
List<Object?> _rowInputs(
  List<MainTimelineEntry> entryGroup,
  List<Object?> rowInputs,
) {
  final summary = entryGroup.first.summary;
  return [
    for (final input in rowInputs)
      if (input is Iterable) ...[input.length, ...input] else input,
    summary?.replyCount,
    summary?.isLowerBound,
    summary?.isCountPending,
    summary?.lastReplyAt,
    ...?summary?.participantPubkeys,
    entryGroup.length,
    DateUtils.dateOnly(DateTime.now()), // Today and Yesterday dividers
    for (final grouped in entryGroup) grouped.message,
  ];
}

bool _sameInputs(List<Object?> a, List<Object?> b) {
  if (a.length != b.length) return false;
  for (var i = 0; i < a.length; i++) {
    if (!identical(a[i], b[i]) && a[i] != b[i]) return false;
  }
  return true;
}
