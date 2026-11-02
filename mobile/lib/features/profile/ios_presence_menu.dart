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
  ModalRoute<Object?>? _route;
  bool _routeSettled = false;
  bool _nativeVisible = false;

  // iOS composites Flutter content painted after a platform view outside
  // every native view rect into the background, beneath earlier overlays.
  // While Home's native bars still show through a route transition, that
  // hides the Settings rows below this pill, so the native view mounts only
  // once its route has settled and a drawn stand-in covers the transitions.
  static bool _isSettled(ModalRoute<Object?>? route) =>
      (route?.animation?.isCompleted ?? true) &&
      (route?.secondaryAnimation?.isDismissed ?? true);

  void _syncRouteSettled([AnimationStatus? _]) {
    final settled = _isSettled(_route);
    if (settled == _routeSettled) return;
    if (!settled) {
      _channel?.setMethodCallHandler(null);
      _channel = null;
      _nativeVisible = false;
    }
    setState(() => _routeSettled = settled);
  }

  void _listenToRoute(ModalRoute<Object?>? route) {
    _route?.animation?.removeStatusListener(_syncRouteSettled);
    _route?.secondaryAnimation?.removeStatusListener(_syncRouteSettled);
    _route = route;
    route?.animation?.addStatusListener(_syncRouteSettled);
    route?.secondaryAnimation?.addStatusListener(_syncRouteSettled);
  }

  Map<String, Object> _configuration(BuildContext context) => {
    'presence': widget.presence,
    'label': widget.label,
    'foreground': widget.foreground.toARGB32(),
    'background': widget.background.toARGB32(),
    'dark': Theme.of(context).brightness == Brightness.dark,
    'fontSize': MediaQuery.textScalerOf(context).scale(15),
  };

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    final route = ModalRoute.of(context);
    if (route != null && identical(route, _route)) return;
    _listenToRoute(route);
    // A newly pushed route can still report a completed animation while it
    // first builds, so decide once that frame has been laid out.
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) _syncRouteSettled();
    });
  }

  @override
  void didUpdateWidget(covariant IosPresenceMenu oldWidget) {
    super.didUpdateWidget(oldWidget);
    unawaited(_channel?.invokeMethod<void>('update', _configuration(context)));
  }

  @override
  void dispose() {
    _listenToRoute(null);
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
      child: Stack(
        fit: StackFit.expand,
        children: [
          if (!_nativeVisible) _standIn(fontSize),
          if (_routeSettled) _nativeMenu(context),
        ],
      ),
    );
  }

  /// Draws the native pill's capsule and title while the native view is
  /// absent, so route transitions and view creation do not blink the pill.
  Widget _standIn(double fontSize) => Semantics(
    label: 'Availability',
    value: widget.label,
    excludeSemantics: true,
    child: Padding(
      padding: const EdgeInsets.symmetric(vertical: 7),
      child: DecoratedBox(
        decoration: ShapeDecoration(
          color: widget.background,
          shape: const StadiumBorder(),
        ),
        child: Center(
          child: Text(
            widget.label,
            maxLines: 1,
            textScaler: TextScaler.noScaling,
            style: TextStyle(
              fontFamily: 'CupertinoSystemText',
              fontSize: fontSize,
              fontWeight: FontWeight.w500,
              color: widget.foreground,
            ),
          ),
        ),
      ),
    ),
  );

  Widget _nativeMenu(BuildContext context) => UiKitView(
    key: const ValueKey('settings-presence-menu'),
    viewType: 'buzz/presence_menu',
    creationParams: _configuration(context),
    creationParamsCodec: const StandardMessageCodec(),
    onPlatformViewCreated: (id) {
      if (!mounted || !_routeSettled) return;
      final channel = MethodChannel('buzz/presence_menu/$id');
      _channel = channel;
      setState(() => _nativeVisible = true);
      channel.setMethodCallHandler((call) async {
        if (!mounted || call.method != 'selected') return;
        final value = call.arguments;
        if (value is String &&
            const ['online', 'away', 'offline'].contains(value)) {
          widget.onSelected(value);
        }
      });
      unawaited(channel.invokeMethod<void>('update', _configuration(context)));
    },
  );
}
