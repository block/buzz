import 'dart:async';
import 'dart:io';

import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:flutter/gestures.dart' show kTouchSlop;
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

/// One UIKit interaction, from its deferred menu request through dismissal.
class ConversationMenuSession {
  ConversationMenuSession({this.bridge});
  final MethodChannel? bridge;
  final labels = Completer<List<String>>();
  final selection = Completer<String?>();
  void close([String? action]) {
    if (!labels.isCompleted) labels.complete([]);
    if (!selection.isCompleted) selection.complete(action);
  }
}

/// Flutter hit-tests the touch once. UIKit has one interaction on the host
/// view, independent of how many conversation rows are built or scrolling.
class _IosConversationMenu {
  static const channel = MethodChannel('buzz/conversation_menu');
  static ValueChanged<ConversationMenuSession>? target;
  static ConversationMenuSession? session;
  static int touchId = 0;
  static final initialized = _initialize();

  static Future<void> _initialize() async {
    channel.setMethodCallHandler((call) async {
      if (call.method == 'prepare') {
        final request = target;
        if (request == null || call.arguments != touchId) return <String>[];
        session?.close();
        final next = ConversationMenuSession(bridge: channel);
        session = next;
        request(next);
        return next.labels.future;
      }
      if (call.method == 'end') {
        session?.close(call.arguments as String?);
        session = null;
      }
      return null;
    });
    await channel.invokeMethod<void>('attach');
  }
}

bool get usesIosConversationMenu => Platform.isIOS;

class IosConversationMenuRow extends HookWidget {
  const IosConversationMenuRow({
    super.key,
    required this.child,
    required this.onMenu,
  });
  final Widget child;
  final ValueChanged<ConversationMenuSession> onMenu;

  @override
  Widget build(BuildContext context) {
    if (!usesIosConversationMenu) return child;
    final latest = useRef(onMenu)..value = onMenu;
    final request = useMemoized(
      () => (ConversationMenuSession session) {
        if (context.mounted) latest.value(session);
      },
    );
    final down = useRef<Offset?>(null);
    void clear() {
      if (down.value == null) return;
      down.value = null;
      if (_IosConversationMenu.target != request) return;
      unawaited(_IosConversationMenu.channel.invokeMethod<void>('disarm'));
    }

    useEffect(() {
      unawaited(_IosConversationMenu.initialized);
      return () {
        clear();
        if (_IosConversationMenu.target == request) {
          _IosConversationMenu.target = null;
          if (_IosConversationMenu.session != null) {
            unawaited(
              _IosConversationMenu.channel.invokeMethod<void>('dismiss'),
            );
          }
        }
      };
    }, const []);
    return Listener(
      behavior: HitTestBehavior.translucent,
      onPointerDown: (event) {
        down.value = event.position;
        final box = context.findRenderObject() as RenderBox;
        final rect = box.localToGlobal(Offset.zero) & box.size;
        _IosConversationMenu.target = request;
        unawaited(
          _IosConversationMenu.channel.invokeMethod<void>('arm', [
            rect.left,
            rect.top,
            rect.width,
            rect.height,
            ++_IosConversationMenu.touchId,
          ]),
        );
      },
      onPointerMove: (event) {
        final start = down.value;
        if (start != null && (event.position - start).distance > kTouchSlop) {
          clear();
        }
      },
      onPointerUp: (_) => clear(),
      onPointerCancel: (_) => clear(),
      child: child,
    );
  }
}

bool get usesAndroidConversationMenu => Platform.isAndroid;
