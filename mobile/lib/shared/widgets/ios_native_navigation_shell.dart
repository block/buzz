import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart' show PlatformViewHitTestBehavior;
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

import 'buzz_navigation_metrics.dart';

/// The current Channel or DM navigation item rendered by the persistent
/// native iOS navigation shell.
class IosNativeNavigationConfiguration {
  const IosNativeNavigationConfiguration({
    required this.title,
    required this.subtitle,
    required this.semanticLabel,
    required this.foregroundColor,
    required this.brightness,
    required this.onBack,
    required this.onTitle,
    this.avatarImageUrl,
    this.avatarFallback,
    this.systemIconName,
    this.showsHuddle = false,
    this.onHuddle,
    this.huddleLabel,
    this.onMembers,
    this.onMore,
  });

  final String title;
  final String subtitle;
  final String semanticLabel;
  final Color foregroundColor;
  final Brightness brightness;
  final String? avatarImageUrl;
  final String? avatarFallback;
  final String? systemIconName;
  final bool showsHuddle;
  final VoidCallback onBack;
  final VoidCallback onTitle;
  final VoidCallback? onHuddle;
  final String? huddleLabel;
  final VoidCallback? onMembers;
  final VoidCallback? onMore;

  String get signature => [
    title,
    subtitle,
    semanticLabel,
    foregroundColor.toARGB32(),
    brightness.name,
    avatarImageUrl,
    avatarFallback,
    systemIconName,
    showsHuddle,
    onHuddle != null,
    huddleLabel,
    onMembers != null,
    onMore != null,
  ].join('|');

  Map<String, Object> toJson() => {
    'visible': true,
    'title': title,
    'subtitle': subtitle,
    'accessibilityLabel': semanticLabel,
    'foregroundColor': foregroundColor.toARGB32(),
    'brightness': brightness.name,
    'avatarImageUrl': ?avatarImageUrl,
    'avatarFallback': ?avatarFallback,
    'systemIconName': ?systemIconName,
    'showsHuddle': showsHuddle,
    'huddleEnabled': onHuddle != null,
    'huddleLabel': ?huddleLabel,
    'showsMembers': onMembers != null,
    'showsMore': onMore != null,
  };
}

class IosNativeNavigationShellController extends ChangeNotifier {
  Object? _owner;
  IosNativeNavigationConfiguration? _configuration;

  IosNativeNavigationConfiguration? get configuration => _configuration;

  void show(Object owner, IosNativeNavigationConfiguration configuration) {
    final changed =
        _owner != owner || _configuration?.signature != configuration.signature;
    _owner = owner;
    _configuration = configuration;
    if (changed) notifyListeners();
  }

  void hide(Object owner) {
    if (_owner != owner) return;
    _owner = null;
    _configuration = null;
    notifyListeners();
  }

  static IosNativeNavigationShellController? maybeOf(BuildContext context) =>
      context
          .dependOnInheritedWidgetOfExactType<_IosNativeNavigationShellScope>()
          ?.controller;
}

class IosNativeNavigationShellHost extends HookWidget {
  const IosNativeNavigationShellHost({super.key, required this.child});

  static const viewType = 'buzz/native_navigation_shell';

  final Widget child;

  @override
  Widget build(BuildContext context) {
    if (defaultTargetPlatform != TargetPlatform.iOS) return child;

    final controller = useMemoized(IosNativeNavigationShellController.new);
    useEffect(() => controller.dispose, [controller]);
    final configuration = useListenable(controller).configuration;
    final channel = useState<MethodChannel?>(null);
    final latestConfiguration = useRef(configuration)..value = configuration;
    final topInset = MediaQuery.paddingOf(context).top;

    useEffect(() {
      final methodChannel = channel.value;
      if (methodChannel == null) return null;
      methodChannel.setMethodCallHandler((call) async {
        final current = latestConfiguration.value;
        if (current == null) return;
        switch (call.method) {
          case 'back':
            current.onBack();
          case 'title':
            current.onTitle();
          case 'huddle':
            current.onHuddle?.call();
          case 'members':
            current.onMembers?.call();
          case 'more':
            current.onMore?.call();
        }
      });
      return () => methodChannel.setMethodCallHandler(null);
    }, [channel.value]);

    useEffect(() {
      final methodChannel = channel.value;
      if (methodChannel == null) return null;
      unawaited(
        methodChannel.invokeMethod<void>(
          'setNavigation',
          configuration?.toJson() ?? const {'visible': false},
        ),
      );
      return null;
    }, [channel.value, configuration?.signature]);

    final shellHeight = topInset + buzzNavigationRowHeight + 1;
    return _IosNativeNavigationShellScope(
      controller: controller,
      child: Stack(
        fit: StackFit.expand,
        children: [
          child,
          Positioned(
            left: 0,
            right: 0,
            top: 0,
            height: shellHeight,
            child: IgnorePointer(
              ignoring: configuration == null,
              child: UiKitView(
                viewType: viewType,
                hitTestBehavior: configuration == null
                    ? PlatformViewHitTestBehavior.transparent
                    : PlatformViewHitTestBehavior.opaque,
                creationParams: <String, Object>{
                  'topInset': topInset,
                  'navigationHeight': buzzNavigationRowHeight,
                },
                creationParamsCodec: const StandardMessageCodec(),
                onPlatformViewCreated: (viewId) {
                  channel.value = MethodChannel('$viewType/$viewId');
                },
              ),
            ),
          ),
        ],
      ),
    );
  }
}

class IosNativeNavigationShellBinding extends HookWidget {
  const IosNativeNavigationShellBinding({
    super.key,
    required this.configuration,
  });

  final IosNativeNavigationConfiguration configuration;

  @override
  Widget build(BuildContext context) {
    final controller = IosNativeNavigationShellController.maybeOf(context);
    final owner = useMemoized(Object.new);
    final latestConfiguration = useRef(configuration)..value = configuration;
    final route = ModalRoute.of(context);

    useEffect(() {
      if (controller == null || route == null) return null;

      void sync() {
        final routeVisible =
            route.animation?.status != AnimationStatus.reverse &&
            route.animation?.status != AnimationStatus.dismissed;
        final uncovered =
            route.secondaryAnimation?.status != AnimationStatus.forward &&
            route.secondaryAnimation?.status != AnimationStatus.completed;
        if (routeVisible && uncovered) {
          controller.show(owner, latestConfiguration.value);
        } else {
          controller.hide(owner);
        }
      }

      void primaryStatusChanged(AnimationStatus _) => sync();
      void secondaryStatusChanged(AnimationStatus _) => sync();

      route.animation?.addStatusListener(primaryStatusChanged);
      route.secondaryAnimation?.addStatusListener(secondaryStatusChanged);
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (context.mounted) sync();
      });
      return () {
        route.animation?.removeStatusListener(primaryStatusChanged);
        route.secondaryAnimation?.removeStatusListener(secondaryStatusChanged);
        controller.hide(owner);
      };
    }, [controller, owner, route]);

    useEffect(() {
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (context.mounted && route?.isCurrent == true) {
          controller?.show(owner, latestConfiguration.value);
        }
      });
      return null;
    }, [controller, owner, route, configuration.signature]);

    return const SizedBox.shrink();
  }
}

class _IosNativeNavigationShellScope extends InheritedWidget {
  const _IosNativeNavigationShellScope({
    required this.controller,
    required super.child,
  });

  final IosNativeNavigationShellController controller;

  @override
  bool updateShouldNotify(_IosNativeNavigationShellScope oldWidget) =>
      controller != oldWidget.controller;
}
