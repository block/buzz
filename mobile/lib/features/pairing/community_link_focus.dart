import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

/// Start link motion with the native keyboard, with a hardware-keyboard fallback.
bool useCommunityLinkFocus(
  BuildContext context, {
  required bool enteringLink,
  required FocusNode focus,
}) {
  final ios = defaultTargetPlatform == TargetPlatform.iOS;
  final reducedMotion = MediaQuery.disableAnimationsOf(context);
  final keyboardReady = useState(false);
  final enteringLinkRef = useRef(enteringLink)..value = enteringLink;
  useEffect(() {
    const channel = MethodChannel('buzz/community_keyboard');
    Timer? fallback;
    var disposed = false;
    void beginOpening() {
      if (disposed || !enteringLinkRef.value || keyboardReady.value) return;
      fallback?.cancel();
      keyboardReady.value = true;
    }

    if (ios && enteringLink) {
      channel.setMethodCallHandler((call) async {
        if (call.method != 'willShow') return;
        beginOpening();
      });
    }
    if (enteringLink) {
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (disposed || !context.mounted || !enteringLinkRef.value) return;
        focus.requestFocus();
        if (!ios ||
            reducedMotion ||
            MediaQuery.viewInsetsOf(context).bottom > 0) {
          beginOpening();
        } else {
          // Hardware keyboards and older hosts may send no show notification.
          fallback = Timer(const Duration(milliseconds: 350), beginOpening);
        }
      });
    } else {
      focus.unfocus();
      keyboardReady.value = false;
    }
    return () {
      disposed = true;
      fallback?.cancel();
      if (ios && enteringLink) channel.setMethodCallHandler(null);
    };
  }, [enteringLink, ios, reducedMotion]);

  return keyboardReady.value;
}
