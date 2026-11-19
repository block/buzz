import 'dart:async';
import 'dart:ui' show FlutterView;

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

/// How long after a resume a restored keyboard is still treated as UIKit's.
const orphanedKeyboardResumeWindow = Duration(seconds: 2);

/// Hides an iOS keyboard that UIKit restores after an app switch while no
/// Flutter text field has focus.
///
/// From iOS 26, UIKit resigns Flutter's text input view when the app leaves
/// the foreground and makes it first responder again on return. The engine
/// reports only the resignation, so every field is unfocused but the keyboard
/// is back, and nothing in the app can dismiss it (flutter/flutter#189051,
/// fixed on Flutter master but not yet in stable). Hiding it here, only in
/// the moments after a resume, leaves every other keyboard decision to the
/// fields themselves.
void useOrphanedKeyboardGuard() {
  final view = View.of(useContext());
  useEffect(() {
    if (defaultTargetPlatform != TargetPlatform.iOS) return null;
    final observer = _OrphanedKeyboardObserver(view);
    WidgetsBinding.instance.addObserver(observer);
    return () {
      WidgetsBinding.instance.removeObserver(observer);
      observer.dispose();
    };
  }, [view]);
}

class _OrphanedKeyboardObserver with WidgetsBindingObserver {
  _OrphanedKeyboardObserver(this.view);

  final FlutterView view;
  Timer? _window;

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    if (state != AppLifecycleState.resumed) {
      dispose();
      return;
    }
    _window?.cancel();
    _window = Timer(orphanedKeyboardResumeWindow, () => _window = null);
    _hideIfOrphaned();
  }

  @override
  void didChangeMetrics() {
    if (_window != null) _hideIfOrphaned();
  }

  void dispose() {
    _window?.cancel();
    _window = null;
  }

  void _hideIfOrphaned() {
    if (view.viewInsets.bottom <= 0 || _textFieldHasFocus()) return;
    unawaited(SystemChannels.textInput.invokeMethod<void>('TextInput.hide'));
  }
}

bool _textFieldHasFocus() =>
    FocusManager.instance.primaryFocus?.context
        ?.findAncestorStateOfType<EditableTextState>() !=
    null;
