import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

/// Native iOS availability pill with a system menu owned by UIKit.
class IosPresenceMenu extends StatefulWidget {
  const IosPresenceMenu({
    super.key,
    required this.presence,
    required this.label,
    required this.foreground,
    required this.background,
    required this.onSelected,
  });

  final String presence;
  final String label;
  final Color foreground;
  final Color background;
  final ValueChanged<String> onSelected;

  @override
  State<IosPresenceMenu> createState() => _IosPresenceMenuState();
}

class _IosPresenceMenuState extends State<IosPresenceMenu> {
  MethodChannel? _channel;

  Map<String, Object> _configuration(BuildContext context) => {
    'presence': widget.presence,
    'label': widget.label,
    'foreground': widget.foreground.toARGB32(),
    'background': widget.background.toARGB32(),
    'dark': Theme.of(context).brightness == Brightness.dark,
    'fontSize': MediaQuery.textScalerOf(context).scale(15),
  };

  @override
  void didUpdateWidget(covariant IosPresenceMenu oldWidget) {
    super.didUpdateWidget(oldWidget);
    unawaited(_channel?.invokeMethod<void>('update', _configuration(context)));
  }

  @override
  void dispose() {
    _channel?.setMethodCallHandler(null);
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final fontSize = MediaQuery.textScalerOf(context).scale(15);
    final text = TextPainter(
      text: TextSpan(
        text: widget.label,
        style: TextStyle(fontSize: fontSize, fontWeight: FontWeight.w500),
      ),
      textDirection: Directionality.of(context),
    )..layout();
    final width = text.width + 32;
    final height = (fontSize + 24).clamp(44.0, double.infinity);
    text.dispose();
    return SizedBox(
      key: const ValueKey('settings-presence-target'),
      width: width,
      height: height,
      child: UiKitView(
        key: const ValueKey('settings-presence-menu'),
        viewType: 'buzz/presence_menu',
        creationParams: _configuration(context),
        creationParamsCodec: const StandardMessageCodec(),
        onPlatformViewCreated: (id) {
          final channel = MethodChannel('buzz/presence_menu/$id');
          _channel = channel;
          channel.setMethodCallHandler((call) async {
            if (!mounted || call.method != 'selected') return;
            final value = call.arguments;
            if (value is String &&
                const ['online', 'away', 'offline'].contains(value)) {
              widget.onSelected(value);
            }
          });
          unawaited(
            channel.invokeMethod<void>('update', _configuration(context)),
          );
        },
      ),
    );
  }
}
