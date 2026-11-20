import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

/// The longest text, in UTF-16 code units, for which an iOS text field keeps
/// stylus handwriting on.
///
/// With handwriting on, Flutter measures every character of the field, one at
/// a time, after each frame whose text layout changed. The cost grows with the
/// square of the length, so a long draft froze the app for over a minute. The
/// same measurements let the keyboard trackpad's two-finger gesture extend a
/// selection, so text up to this length keeps them.
const stylusHandwritingMaxLength = 2000;

/// Whether a text field holding [text] should keep stylus handwriting on.
///
/// Android doesn't take these measurements, so it keeps stylus handwriting on
/// at any length.
bool stylusHandwritingEnabledFor(String text) =>
    defaultTargetPlatform != TargetPlatform.iOS ||
    text.length <= stylusHandwritingMaxLength;

/// Whether [controller]'s field should keep stylus handwriting on, updated
/// only when its text crosses [stylusHandwritingMaxLength].
///
/// Flutter stops sending character rectangles once handwriting turns off, but
/// iOS keeps the last ones it received, so the two-finger trackpad gesture and
/// the autocorrect highlight would use positions from the shorter text. When
/// the text grows past the cutoff while [focusNode] has focus, this clears
/// them. iOS holds rectangles only for the focused field, so a field crossing
/// the cutoff without focus has none to clear.
class StylusHandwritingCutoff extends ValueNotifier<bool> {
  StylusHandwritingCutoff(this._controller, this._focusNode)
    : super(stylusHandwritingEnabledFor(_controller.text)) {
    _controller.addListener(_update);
  }

  final TextEditingController _controller;
  final FocusNode _focusNode;

  void _update() {
    final enabled = stylusHandwritingEnabledFor(_controller.text);
    if (value && !enabled && _focusNode.hasFocus) {
      SystemChannels.textInput.invokeMethod<void>(
        'TextInput.setSelectionRects',
        const <Object>[],
      );
    }
    value = enabled;
  }

  @override
  void dispose() {
    _controller.removeListener(_update);
    super.dispose();
  }
}
