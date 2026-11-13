part of '../compose_bar.dart';

// Hardware keyboards only: software Return keeps its newline. While the
// suggestion picker is shown, Tab and Enter accept its first selectable row
// (or do nothing when it has none); otherwise Enter sends and Tab traverses.
// Shift+Enter leaves newline handling to the field.
KeyEventResult _handleComposerHardwareKey(
  KeyEvent event,
  TextEditingController controller, {
  required bool pickerVisible,
  required VoidCallback? acceptSuggestion,
  required Future<void> Function() send,
}) {
  if (event is KeyUpEvent) return KeyEventResult.ignored;
  final keyboard = HardwareKeyboard.instance;
  final key = event.logicalKey;
  if (key == LogicalKeyboardKey.tab &&
      !keyboard.isShiftPressed &&
      !keyboard.isControlPressed &&
      !keyboard.isAltPressed &&
      !keyboard.isMetaPressed) {
    if (!pickerVisible) return KeyEventResult.ignored;
    if (event is KeyDownEvent) acceptSuggestion?.call();
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
  // Android IMEs can emit Enter KeyEvents without a physical scan code.
  // Flutter gives those synthetic Android-plane keys, not USB Enter keys.
  if (defaultTargetPlatform == TargetPlatform.android &&
      event.physicalKey != PhysicalKeyboardKey.enter &&
      event.physicalKey != PhysicalKeyboardKey.numpadEnter) {
    return KeyEventResult.ignored;
  }
  if (event is KeyDownEvent) {
    if (pickerVisible) {
      acceptSuggestion?.call();
    } else {
      unawaited(send());
    }
  }
  return KeyEventResult.handled;
}
