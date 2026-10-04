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
    this.leading,
    this.actions = const [],
    this.onBack,
    this.foregroundColor,
  });

  static const viewType = 'buzz/ios_navigation_bar';

  final String title;
  final String? subtitle;
  final String? ephemeralLabel;
  final IosNavigationAction? titleAvatar;
  final Color? titlePresenceColor;
  final VoidCallback? onTitlePressed;
  final bool largeTitle;
  final IosNavigationAction? leading;
  final List<IosNavigationAction> actions;
  final VoidCallback? onBack;
  final Color? foregroundColor;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final channel = useState<MethodChannel?>(null);
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
    final retainedImages = useRef(<String, ({String key, String data})>{});
    final avatarKey = jsonEncode([
      for (final entry in avatarActions.entries)
        [entry.key, imageKey(entry.value)],
    ]);
    final avatarFuture = useMemoized(
      () async {
        final images = <String, String>{};
        await Future.wait(
          avatarActions.entries.map((entry) async {
            final bytes = await nativeAvatarImage(
              url: entry.value.imageUrl,
              initial: entry.value.avatarInitial!,
              isAgent: entry.value.avatarIsAgent,
              background: colors.primaryContainer,
              foreground: colors.onPrimaryContainer,
              networkImage: (url) =>
                  MediaImageProvider(url: url, auth: auth, client: client),
            );
            if (bytes != null) images[entry.key] = base64Encode(bytes);
          }),
        );
        return images;
      },
      [
        avatarKey,
        auth,
        client,
        colors.primaryContainer,
        colors.onPrimaryContainer,
      ],
    );
    final avatarImages = useFuture(avatarFuture, preserveState: false).data;
    Map<String, Object?> encodeAction(IosNavigationAction action, String id) {
      final key = imageKey(action);
      final image = avatarImages?[id];
      if (image != null) retainedImages.value[id] = (key: key, data: image);
      final retained = retainedImages.value[id];
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
      if (current == null) return null;
      unawaited(current.invokeMethod<void>('configure', payload));
      return null;
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
      viewType: viewType,
      creationParams: payload,
      creationParamsCodec: const StandardMessageCodec(),
      onPlatformViewCreated: (id) {
        channel.value = MethodChannel('$viewType/$id');
      },
    );
  }
}
