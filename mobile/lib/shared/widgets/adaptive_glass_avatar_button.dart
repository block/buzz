import 'dart:ui';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../theme/theme.dart';
import 'avatar_image.dart';
import 'ios_glass_navigation_button.dart';
import 'native_avatar_data_uri_provider.dart';

/// A compact avatar control that uses native Liquid Glass on iOS and the
/// matching composited glass treatment on other platforms.
class AdaptiveGlassAvatarButton extends ConsumerWidget {
  const AdaptiveGlassAvatarButton({
    super.key,
    required this.imageUrl,
    required this.fallbackText,
    required this.semanticLabel,
    required this.onPressed,
    required this.width,
    this.label,
    this.iosMenuItems = const [],
    this.onIosMenuSelected,
    this.nativeViewSuppressed,
  });

  final String? imageUrl;
  final String fallbackText;
  final String semanticLabel;
  final VoidCallback onPressed;
  final double width;
  final String? label;
  final List<IosGlassNavigationMenuItem> iosMenuItems;
  final ValueChanged<String>? onIosMenuSelected;
  final ValueListenable<bool>? nativeViewSuppressed;

  static const double height = 48;
  static const double avatarSize = 36;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    if (defaultTargetPlatform == TargetPlatform.iOS) {
      final source = imageUrl;
      final nativeImage = source == null
          ? null
          : ref.watch(nativeAvatarDataUriProvider(source)).value ?? source;
      return IosGlassNavigationButton(
        icon: IosGlassNavigationIcon.avatar,
        label: label,
        semanticLabel: semanticLabel,
        onPressed: onPressed,
        width: width,
        height: height,
        controlSize: height,
        fillWidth: true,
        foregroundColor: context.colors.onSurface,
        avatarImageUrl: nativeImage,
        avatarFallback: fallbackText,
        menuItems: iosMenuItems,
        onMenuSelected: onIosMenuSelected,
        nativeViewSuppressed: nativeViewSuppressed,
      );
    }

    final radius = BorderRadius.circular(height / 2);
    return Semantics(
      button: true,
      label: semanticLabel,
      child: ClipRRect(
        borderRadius: radius,
        child: BackdropFilter(
          filter: ImageFilter.blur(sigmaX: 18, sigmaY: 18),
          child: Material(
            color: context.colors.surface.withValues(alpha: 0.68),
            child: InkWell(
              onTap: onPressed,
              child: Container(
                width: width,
                height: height,
                // The 1dp border participates in layout, so 5dp of content
                // padding leaves the 36dp avatar exactly 6dp from every edge.
                padding: const EdgeInsets.all(5),
                decoration: BoxDecoration(
                  borderRadius: radius,
                  border: Border.all(
                    color: context.colors.inverseSurface.withValues(
                      alpha: 0.08,
                    ),
                  ),
                ),
                child: Stack(
                  alignment: Alignment.centerLeft,
                  children: [
                    AvatarImage(
                      imageUrl: imageUrl,
                      radius: avatarSize / 2,
                      backgroundColor: context.colors.primaryContainer,
                      fallback: Text(
                        fallbackText,
                        style: context.textTheme.labelMedium?.copyWith(
                          color: context.colors.onPrimaryContainer,
                          fontWeight: FontWeight.w600,
                        ),
                      ),
                    ),
                    if (label != null && width >= 60)
                      Positioned(
                        left: avatarSize + Grid.xxs,
                        right: Grid.xxs,
                        child: Text(
                          label!,
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis,
                          style: context.textTheme.titleMedium?.copyWith(
                            color: navigationPrimaryForeground(context),
                            fontWeight: FontWeight.w600,
                          ),
                        ),
                      ),
                  ],
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }
}
