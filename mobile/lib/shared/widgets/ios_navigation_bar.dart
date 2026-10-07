import 'dart:async';
import 'dart:convert';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../relay/relay.dart';
import 'avatar_image.dart';
import 'ios_navigation_metrics.dart';
export 'ios_navigation_metrics.dart';

/// A UIKit bar item or menu item, with its action owned by the Flutter route.
class IosNavigationAction {
  const IosNavigationAction({
    required this.label,
    this.symbol,
    this.imageUrl,
    this.avatarInitial,
    this.avatarIdentity,
    this.avatarIsAgent = false,
    this.activityColor,
    this.activityLabel,
    this.onPressed,
    this.children = const [],
    this.selected = false,
    this.onAvatarBoundsChanged,
    this.avatarHidden = false,

    this.plain = false,
  });

  final String label;
  final String? symbol;
  final String? imageUrl;
  final String? avatarInitial;
  final String? avatarIdentity;
  final bool avatarIsAgent;
  final Color? activityColor;
  final String? activityLabel;
  final VoidCallback? onPressed;
  final List<IosNavigationAction> children;
  final bool selected;

  /// Reports the avatar's global bounds for a transition into this bar item.
  final ValueChanged<Rect>? onAvatarBoundsChanged;

  /// Hides a transition's destination avatar while retaining its layout slot.
  final bool avatarHidden;

  /// Omits the shared Liquid Glass background for a text-only action.
  final bool plain;

  Map<String, Object?> _encode(String id) => {
    'id': id,
    'label': label,
    'symbol': avatarInitial == null ? symbol : null,
    'avatarInitial': avatarInitial,
    'avatarIsAgent': avatarIsAgent,
    'activityColor': activityColor?.toARGB32(),
    'activityLabel': activityLabel,
    'imageUrl': imageUrl,
    'enabled': onPressed != null || children.isNotEmpty,
    'selected': selected,
    'tracksAvatarBounds': onAvatarBoundsChanged != null,
    'avatarHidden': avatarHidden,

    'plain': plain,
    'children': [
      for (var i = 0; i < children.length; i++) children[i]._encode('$id.$i'),
    ],
  };

  void _dispatch(List<String> path) {
    if (path.isEmpty) {
      onPressed?.call();
      return;
    }
    final index = int.tryParse(path.first);
    if (index != null && index >= 0 && index < children.length) {
      children[index]._dispatch(path.sublist(1));
    }
  }
}

/// Scroll position of the route whose UIKit navigation bar is visible.
class IosNavigationScrollScope extends InheritedWidget {
  const IosNavigationScrollScope({
    super.key,
    required this.offset,
    required super.child,
  });

  final ValueListenable<double> offset;

  static ValueListenable<double>? maybeOf(BuildContext context) => context
      .dependOnInheritedWidgetOfExactType<IosNavigationScrollScope>()
      ?.offset;

  @override
  bool updateShouldNotify(IosNavigationScrollScope oldWidget) =>
      offset != oldWidget.offset;
}

/// A real UINavigationController bar embedded in the current Flutter route.
/// UIKit owns title layout, SF Symbols, menus, back items, and large-title motion.
class IosNavigationBar extends HookConsumerWidget {
  const IosNavigationBar({
    super.key,
    required this.title,
    this.subtitle,
    this.ephemeralLabel,
    this.titleAvatar,
    this.titlePresenceColor,
    this.onTitlePressed,
    this.largeTitle = false,
    this.alwaysFrosted = false,
    this.leading,
    this.actions = const [],
    this.onBack,
    this.foregroundColor,
    this.onReadyChanged,
  });

  static const viewType = 'buzz/ios_navigation_bar';

  final String title;
  final String? subtitle;
  final String? ephemeralLabel;
  final IosNavigationAction? titleAvatar;
  final Color? titlePresenceColor;
  final VoidCallback? onTitlePressed;
  final bool largeTitle;

  /// Keeps the navigation backdrop visible before the first scroll update.
  final bool alwaysFrosted;
  final IosNavigationAction? leading;
  final List<IosNavigationAction> actions;
  final VoidCallback? onBack;
  final Color? foregroundColor;

  /// Reports when the latest native configuration and layout are applied.
  /// Cosmetic avatar downloads do not delay readiness.
  final ValueChanged<bool>? onReadyChanged;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final channel = useState<MethodChannel?>(null);
    final viewKey = useMemoized(GlobalKey.new);
    final latest = useRef(this)..value = this;
    final offset = IosNavigationScrollScope.maybeOf(context);
    final collapseRange = IosNavigationMetrics.of(context).largeTitleHeight;
    final avatarActions = <String, IosNavigationAction>{
      'titleAvatar': ?titleAvatar,
      if (leading?.avatarInitial != null) 'leading': leading!,
      for (var i = 0; i < actions.length; i++)
        if (actions[i].avatarInitial != null) '$i': actions[i],
    };
    final auth = ref.watch(mediaGetAuthServiceProvider);
    final client = ref.watch(mediaHttpClientProvider);
    final colors = Theme.of(context).colorScheme;
    String imageKey(IosNavigationAction action) => jsonEncode([
      action.avatarIdentity,
      action.imageUrl,
      action.avatarInitial,
      action.avatarIsAgent,
    ]);
    final avatarImages = useState(<String, ({String key, String data})>{});
    final avatarKey = jsonEncode([
      for (final entry in avatarActions.entries)
        [entry.key, imageKey(entry.value)],
    ]);
    useEffect(
      () {
        var active = true;
        for (final entry in avatarActions.entries) {
          unawaited(
            nativeAvatarImage(
              url: entry.value.imageUrl,
              initial: entry.value.avatarInitial!,
              isAgent: entry.value.avatarIsAgent,
              background: colors.primaryContainer,
              foreground: colors.onPrimaryContainer,
              networkImage: (url) =>
                  MediaImageProvider(url: url, auth: auth, client: client),
            ).then(
              (bytes) {
                if (!active || !context.mounted || bytes == null) return;
                avatarImages.value = {
                  ...avatarImages.value,
                  entry.key: (
                    key: imageKey(entry.value),
                    data: base64Encode(bytes),
                  ),
                };
              },
              onError: (Object _, StackTrace _) {
                // Keep UIKit's synchronous initial fallback on image failure.
              },
            ),
          );
        }
        return () => active = false;
      },
      [
        avatarKey,
        auth,
        client,
        colors.primaryContainer,
        colors.onPrimaryContainer,
      ],
    );
    Map<String, Object?> encodeAction(IosNavigationAction action, String id) {
      final key = imageKey(action);
      final retained = avatarImages.value[id];
      return {
        ...action._encode(id),
        'avatarBackground': colors.primaryContainer.toARGB32(),
        'avatarForeground': colors.onPrimaryContainer.toARGB32(),
        if (retained?.key == key) 'imageData': retained!.data,
      };
    }

    final payload = <String, Object?>{
      'title': title,
      'subtitle': subtitle,
      'ephemeralLabel': ephemeralLabel,
      'titleAvatar': titleAvatar == null
          ? null
          : encodeAction(titleAvatar!, 'titleAvatar'),
      'titlePresenceColor': titlePresenceColor?.toARGB32(),
      'titleEnabled': onTitlePressed != null,
      'largeTitle': largeTitle,
      'alwaysFrosted': alwaysFrosted,
      'back': onBack != null,
      'leading': leading == null ? null : encodeAction(leading!, 'leading'),
      'actions': [
        for (var i = 0; i < actions.length; i++) encodeAction(actions[i], '$i'),
      ],
      'background': colors.surface.toARGB32(),
      'dark': Theme.of(context).brightness == Brightness.dark,
      'foreground': (foregroundColor ?? Theme.of(context).colorScheme.onSurface)
          .toARGB32(),
    };
    final signature = jsonEncode(payload);

    useEffect(() {
      final current = channel.value;
      if (current == null) return null;
      current.setMethodCallHandler((call) async {
        if (call.method == 'avatarBounds') {
          final values = call.arguments as Map<Object?, Object?>;
          final box = viewKey.currentContext?.findRenderObject();
          if (box is RenderBox && values['id'] == 'leading') {
            final x = (values['x'] as num).toDouble();
            final y = (values['y'] as num).toDouble();
            final width = (values['width'] as num).toDouble();
            final height = (values['height'] as num).toDouble();
            latest.value.leading?.onAvatarBoundsChanged?.call(
              box.localToGlobal(Offset(x, y)) & Size(width, height),
            );
          }
          return;
        }
        if (call.method == 'metrics') {
          if (context.mounted) {
            IosNavigationMetrics.update(
              context,
              call.arguments as Map<Object?, Object?>,
            );
          }
          return;
        }
        if (call.method != 'action') return;
        final id = call.arguments as String;
        final config = latest.value;
        if (id == 'title') {
          config.onTitlePressed?.call();
        } else if (id == 'back') {
          config.onBack?.call();
        } else {
          final path = id.split('.');
          if (path.first == 'leading') {
            config.leading?._dispatch(path.sublist(1));
          } else {
            final index = int.tryParse(path.first);
            if (index != null && index >= 0 && index < config.actions.length) {
              config.actions[index]._dispatch(path.sublist(1));
            }
          }
        }
      });
      return () => current.setMethodCallHandler(null);
    }, [channel.value]);

    useEffect(() {
      final current = channel.value;
      var active = true;
      void report(bool ready) {
        WidgetsBinding.instance.addPostFrameCallback((_) {
          if (active && context.mounted) {
            latest.value.onReadyChanged?.call(ready);
          }
        });
        WidgetsBinding.instance.ensureVisualUpdate();
      }

      report(false);
      if (current != null) {
        unawaited(() async {
          try {
            await current.invokeMethod<void>('configure', payload);
            if (!active) return;
            if (latest.value.onReadyChanged != null) {
              await current.invokeMethod<void>('prepareForReveal');
            }
            report(true);
          } on PlatformException catch (error) {
            debugPrint('Native navigation configuration failed: $error');
          }
        }());
      }
      return () => active = false;
    }, [channel.value, signature]);

    useEffect(() {
      void sync() {
        final current = channel.value;
        if (current != null) {
          unawaited(
            current.invokeMethod<void>(
              'scroll',
              (offset?.value ?? 0).clamp(0.0, collapseRange),
            ),
          );
        }
      }

      sync();
      offset?.addListener(sync);
      return () => offset?.removeListener(sync);
    }, [channel.value, offset, collapseRange]);

    return UiKitView(
      key: viewKey,
      viewType: viewType,
      creationParams: payload,
      creationParamsCodec: const StandardMessageCodec(),
      onPlatformViewCreated: (id) {
        channel.value = MethodChannel('$viewType/$id');
      },
    );
  }
}
