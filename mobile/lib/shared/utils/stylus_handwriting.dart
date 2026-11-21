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
/// the text grows past the cutoff while [focusNode] has primary focus, this clears
/// them. iOS holds rectangles only for the focused field, so a field crossing
/// the cutoff without focus has none to clear.
///
/// On any crossing, focused or not, it first closes an open iPad Scribble
/// writing gap. Flutter ignores iOS's request to close the gap once
/// handwriting is off, so the gap would otherwise stay in the text until the
/// field is rebuilt. A gap can outlive focus when iOS closes the input
/// connection itself.
class StylusHandwritingCutoff extends ValueNotifier<bool> {
  StylusHandwritingCutoff(this._controller, this._focusNode)
    : super(stylusHandwritingEnabledFor(_controller.text)) {
    _controller.addListener(_update);
  }

  final TextEditingController _controller;
  final FocusNode _focusNode;

  void _update() {
    final enabled = stylusHandwritingEnabledFor(_controller.text);
    // The node keeps the context of a field that has since been removed, which
    // took its gap with it.
    final fieldContext = _focusNode.context;
    if (value && !enabled && fieldContext != null && fieldContext.mounted) {
      // The field still has handwriting on until it rebuilds, so it accepts
      // the close.
      fieldContext
          .findAncestorStateOfType<EditableTextState>()
          ?.removeTextPlaceholder();
    }
    if (value && !enabled && _focusNode.hasPrimaryFocus) {
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
