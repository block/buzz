part of '../compose_bar.dart';

/// The bindings [_MarkdownEditingController] held with each text it has
/// shown: recorded on every edit, and read back when Undo, Redo or a plain
/// text change returns the draft to one of those texts.
extension _ComposerBindingHistory on _MarkdownEditingController {
  /// Notes the bindings now held with [text] as the newest history entry,
  /// dropping any entries a Redo could have reached.
  void _recordHistory(String text) {
    if (_historyAt >= 0 && _history[_historyAt].text == text) {
      _history[_historyAt] = (text: text, mentions: _mentions);
      return;
    }
    _history.removeRange(_historyAt + 1, _history.length);
    _history.add((text: text, mentions: _mentions));
    if (_history.length > _MarkdownEditingController._maxHistory) {
      // Only an entry with a binding can give one back, so those stay
      // longest.
      final unbound = _history.indexWhere((entry) => entry.mentions.isEmpty);
      _history.removeAt(
        unbound >= 0 && unbound < _history.length - 1 ? unbound : 0,
      );
    }
    _historyAt = _history.length - 1;
  }

  /// Gives a text change that returns the draft to a text it held before
  /// the bindings it held then. The system's own Undo and Redo (an iOS shake
  /// or gesture) arrive as plain text changes and never reach
  /// [_ComposerHistoryAction]: a chip a name character had unbound, or one
  /// that was deleted, would come back as `@Name` with no recipient, and a
  /// send would leave a person outside the channel out without a prompt or
  /// notify a member of the same name instead.
  ///
  /// The nearest entry for [next]'s text is taken when it holds every
  /// binding carried across the edit and more, so nothing the edit kept is
  /// contradicted. One character typed inside a name never completes a
  /// pick: `@Bob` typed by hand after its chip was deleted stays typed
  /// text. Undo cannot be that edit, since a deletion inside a chip always
  /// takes the whole chip. False when nothing was recovered.
  bool _recoverHistory(TextEditingValue old, TextEditingValue next) {
    final text = next.text;
    if (!text.contains('@')) return false;
    bool same(_ComposerMention a, _ComposerMention b) =>
        a.start == b.start && a.end == b.end && a.pubkey == b.pubkey;
    final replaced = editReplacedRange(
      old.text,
      text,
      oldSelection: old.selection,
      newSelection: next.selection,
    );
    final inserted = text.length - old.text.length;
    final typedAt =
        replaced.isCollapsed &&
            inserted > 0 &&
            text
                    .substring(replaced.start, replaced.start + inserted)
                    .runes
                    .length ==
                1
        ? replaced.start
        : null;
    for (var distance = 0; distance < _history.length; distance++) {
      for (final i in {_historyAt - distance, _historyAt + distance}) {
        if (i < 0 || i >= _history.length || _history[i].text != text) {
          continue;
        }
        final held = _history[i].mentions;
        if (held.length <= _mentions.length ||
            !_mentions.every((kept) => held.any((m) => same(m, kept)))) {
          continue;
        }
        if (typedAt != null &&
            held.any(
              (m) =>
                  m.start <= typedAt &&
                  typedAt < m.end &&
                  !_mentions.any((kept) => same(m, kept)),
            )) {
          return false;
        }
        _historyAt = i;
        _mentions = held;
        _cachedTextSpan = null;
        return true;
      }
    }
    return false;
  }

  /// Takes the bindings of the nearest entry for [text] in the direction
  /// Undo or Redo is moving. False when the history holds none, as after it
  /// was trimmed; the caller then carries the bindings as for any edit.
  bool _restoreHistory(String text) {
    for (
      var i = _historyAt + _historyDirection;
      i >= 0 && i < _history.length;
      i += _historyDirection
    ) {
      if (_history[i].text != text) continue;
      _historyAt = i;
      _mentions = _history[i].mentions;
      _cachedTextSpan = null;
      return true;
    }
    return false;
  }
}
