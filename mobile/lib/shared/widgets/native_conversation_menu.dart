import 'dart:async';
import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:flutter/gestures.dart';
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

class IosConversationMenuRow extends HookWidget {
  const IosConversationMenuRow({
    super.key,
    required this.child,
    required this.onTap,
    required this.onMenu,
  });
  final Widget child;
  final VoidCallback onTap;
  final ValueChanged<ConversationMenuSession> onMenu;

  @override
  Widget build(BuildContext context) {
    if (!Platform.isIOS) return child;
    final bridge = useRef<MethodChannel?>(null);
    final session = useRef<ConversationMenuSession?>(null);
    final tapped = useRef(onTap)..value = onTap;
    final requested = useRef(onMenu)..value = onMenu;
    useEffect(
      () => () {
        bridge.value?.setMethodCallHandler(null);
        session.value?.close();
      },
      const [],
    );
    void bind(int id) {
      final channel = MethodChannel('buzz/conversation_row/$id');
      bridge.value = channel;
      channel.setMethodCallHandler((call) async {
        if (!context.mounted) return null;
        switch (call.method) {
          case 'tap':
            tapped.value();
          case 'prepare':
            session.value?.close();
            final next = ConversationMenuSession(bridge: channel);
            session.value = next;
            requested.value(next);
            return next.labels.future;
          case 'end':
            session.value?.close(call.arguments as String?);
            session.value = null;
        }
        return null;
      });
    }

    return Stack(
      children: [
        child,
        Positioned.fill(
          child: ExcludeSemantics(
            child: UiKitView(
              viewType: 'buzz/conversation_row',
              onPlatformViewCreated: bind,
              gestureRecognizers: {
                Factory<TapGestureRecognizer>(TapGestureRecognizer.new),
                Factory<LongPressGestureRecognizer>(
                  LongPressGestureRecognizer.new,
                ),
              },
            ),
          ),
        ),
      ],
    );
  }
}

bool get usesAndroidConversationMenu => Platform.isAndroid;
