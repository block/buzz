import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import '../theme/utility_surface_theme.dart';

const _channel = MethodChannel('buzz/profile_sheet');
int _nextId = 0;
final _sessions =
    <
      int,
      ({
        VoidCallback dismiss,
        ValueNotifier<bool> expanded,
        ValueNotifier<double> offset,
      })
    >{};

class NativeProfileSheet extends InheritedWidget {
  const NativeProfileSheet({
    required this.id,
    required this.expanded,
    required this.offset,
    this.rosterKey,
    this.focused = true,
    required super.child,
    super.key,
  });

  final bool focused;
  final int id;
  final GlobalKey? rosterKey;
  final bool expanded;
  final ValueNotifier<double> offset;

  static NativeProfileSheet? maybeOf(BuildContext context) =>
      context.dependOnInheritedWidgetOfExactType<NativeProfileSheet>();

  Future<void> setPhotoOpen(bool open) =>
      _channel.invokeMethod('photo', {'id': id, 'open': open});

  Future<void> resetScroll() =>
      _channel.invokeMethod('resetScroll', {'id': id});

  Future<void> expand() => _channel.invokeMethod('expand', {'id': id});

  Future<void> setDimensions(
    double height,
    double document,
    bool active, {
    required bool showGrabber,
    Rect? roster,
  }) => _channel.invokeMethod('height', {
    'id': id,
    'height': height,
    'document': document,
    'active': active,
    'roster': roster == null
        ? null
        : [roster.left, roster.top, roster.width, roster.height],
    'showGrabber': showGrabber,
  });

  @override
  bool updateShouldNotify(NativeProfileSheet oldWidget) =>
      oldWidget.focused != focused ||
      oldWidget.expanded != expanded ||
      oldWidget.id != id;
}

Future<T?> showNativeProfileSheet<T>({
  required BuildContext context,
  bool initiallyExpanded = false,
  required WidgetBuilder builder,
  required Future<T?> Function() fallback,
}) async {
  final navigator = Navigator.of(context, rootNavigator: true);
  final id = ++_nextId;
  final rosterKey = GlobalKey();
  final offset = ValueNotifier(0.0);
  final expanded = ValueNotifier(initiallyExpanded);
  final route = PageRouteBuilder<T>(
    transitionDuration: Duration.zero,
    reverseTransitionDuration: Duration.zero,
    pageBuilder: (_, _, _) => ValueListenableBuilder<bool>(
      valueListenable: expanded,
      builder: (routeContext, large, child) => NativeProfileSheet(
        id: id,
        expanded: large,
        offset: offset,
        rosterKey: rosterKey,
        child: Material(
          color: utilitySurfaceThemeData(
            Theme.of(routeContext),
          ).scaffoldBackgroundColor,
          child: child,
        ),
      ),
      child: Builder(builder: builder),
    ),
  );
  _channel.setMethodCallHandler((call) async {
    final args = call.arguments as Map;
    final session = _sessions[args['id']];
    if (call.method == 'dismissed') session?.dismiss();
    if (call.method == 'scroll') {
      session?.offset.value = (args['offset'] as num).toDouble();
    }
    if (call.method == 'expanded') {
      session?.expanded.value = args['expanded'] == true;
    }
  });
  _sessions[id] = (
    dismiss: () {
      if (route.isActive) navigator.removeRoute(route);
    },
    expanded: expanded,
    offset: offset,
  );
  var presented = false;
  try {
    try {
      presented =
          await _channel.invokeMethod<bool>('prepare', {
            'id': id,
            'expanded': initiallyExpanded,
          }) ??
          false;
    } on MissingPluginException {
      presented = false;
    }
    if (!context.mounted || !navigator.mounted) return null;
    if (presented) {
      final closed = navigator.push(route);
      if (!route.isActive) return await closed;
      var shown = false;
      try {
        shown =
            await _channel.invokeMethod<bool>('present', {'id': id}) ?? false;
      } on PlatformException {
        // UIKit can reject presentation while another controller is transitioning.
      }
      if (shown || !route.isActive) return await closed;
    }
  } finally {
    if (route.isActive) navigator.removeRoute(route);
    _sessions.remove(id);
    if (presented) await _channel.invokeMethod<void>('dismiss', {'id': id});
    expanded.dispose();
    offset.dispose();
  }
  return context.mounted ? await fallback() : null;
}
