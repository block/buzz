part of '../compose_bar.dart';

// Hardware keyboards only: software Return keeps its newline. Tab accepts a
// mention, Enter sends, and Shift+Enter leaves newline handling to the field.
KeyEventResult _handleComposerHardwareKey(
  KeyEvent event,
  TextEditingController controller,
  MentionCandidate? top,
  void Function(MentionCandidate) insertMention,
  Future<void> Function() send,
) {
  if (event is KeyUpEvent) return KeyEventResult.ignored;
  final keyboard = HardwareKeyboard.instance;
  final key = event.logicalKey;
  if (key == LogicalKeyboardKey.tab && !keyboard.isShiftPressed) {
    if (top == null) {
      return KeyEventResult.ignored;
    }
    if (event is KeyDownEvent) insertMention(top);
    return KeyEventResult.handled;
  }
  final isEnter =
      key == LogicalKeyboardKey.enter || key == LogicalKeyboardKey.numpadEnter;
  // A composing IME owns Enter: it commits the candidate text.
  if (!isEnter ||
      keyboard.isShiftPressed ||
      keyboard.isControlPressed ||
      keyboard.isAltPressed ||
      keyboard.isMetaPressed ||
      !controller.value.composing.isCollapsed) {
    return KeyEventResult.ignored;
  }
  if (event is KeyDownEvent) unawaited(send());
  return KeyEventResult.handled;
}
