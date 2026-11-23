part of '../compose_bar.dart';

/// The bindings [_MarkdownEditingController] held with each text it has
/// shown in the current draft: recorded on every edit, and read back only
/// when a confirmed Undo or Redo returns the draft to one of those texts
/// ([_ComposerHistoryAction], [_ComposerPlatformUndoClient]). A plain text
/// change never reads it: a paste or a retyped name that lands on an
/// earlier text is typed text, not that pick again.
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

/// Marks the system's own Undo and Redo (an iOS shake or gesture) for
/// [_MarkdownEditingController]. The platform reports them to
/// [UndoManager.client], which calls the field's `UndoHistory` directly and
/// never passes [_ComposerHistoryAction]; the restored text then reaches
/// the controller looking like any other edit. This client stands in front
/// of the field's own and says which way the history is moving while that
/// one restores the value, exactly as the action does for the intents.
class _ComposerPlatformUndoClient with UndoManagerClient {
  final UndoManagerClient field;
  final _MarkdownEditingController controller;

  _ComposerPlatformUndoClient(this.field, this.controller);

  @override
  void handlePlatformUndo(UndoDirection direction) {
    controller._historyDirection = direction == UndoDirection.redo ? 1 : -1;
    try {
      field.handlePlatformUndo(direction);
    } finally {
      controller._historyDirection = 0;
    }
  }

  @override
  void undo() => field.undo();

  @override
  void redo() => field.redo();

  @override
  bool get canUndo => field.canUndo;

  @override
  bool get canRedo => field.canRedo;

  /// Equal to the client it stands for. `UndoHistory` asks whether
  /// [UndoManager.client] is itself before it reports its undo state to
  /// iOS and before it releases the client on blur and dispose.
  @override
  bool operator ==(Object other) =>
      identical(other, this) ||
      identical(other, field) ||
      other is _ComposerPlatformUndoClient && identical(other.field, field);

  @override
  int get hashCode => identityHashCode(field);
}

/// Puts a [_ComposerPlatformUndoClient] in front of the undo client of the
/// field that [focusNode] focuses, for as long as it has focus. The field's
/// `UndoHistory` names itself the client on every focus notification, after
/// this listener has run, so the wrap waits for the listeners to finish.
/// Without the wrap a system Undo restores text only, as a plain edit.
void _usePlatformUndoBindings(
  _MarkdownEditingController controller,
  FocusNode focusNode,
) {
  useEffect(() {
    var disposed = false;
    void wrap() {
      if (disposed || !focusNode.hasFocus) return;
      final client = UndoManager.client;
      if (client == null || client is _ComposerPlatformUndoClient) return;
      UndoManager.client = _ComposerPlatformUndoClient(client, controller);
    }

    void onFocus() => scheduleMicrotask(wrap);
    focusNode.addListener(onFocus);
    onFocus();
    return () {
      disposed = true;
      focusNode.removeListener(onFocus);
    };
  }, [controller, focusNode]);
}
