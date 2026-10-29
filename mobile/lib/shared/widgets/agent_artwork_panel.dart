import 'dart:ui';
import 'package:flutter/material.dart';
import '../animated_avatar.dart';
import '../theme/theme.dart';
import 'avatar_image.dart';

/// A desktop-style profile container with static, faded portrait artwork.
class AgentArtworkPanel extends StatelessWidget {
  const AgentArtworkPanel({
    required this.imageUrl,
    required this.child,
    this.borderRadius,
    super.key,
  });
  final String? imageUrl;
  final Widget child;
  final BorderRadius? borderRadius;

  @override
  Widget build(BuildContext context) {
    final poster = parseAnimatedAvatarUrl(imageUrl)?.posterUrl ?? imageUrl;
    return ClipRRect(
      borderRadius: borderRadius ?? BorderRadius.circular(Radii.card),
      child: ColoredBox(
        color: context.colors.surfaceContainerLow,
        child: Stack(
          children: [
            if (poster != null && !MediaQuery.highContrastOf(context))
              Positioned.fill(
                child: IgnorePointer(
                  child: ExcludeSemantics(
                    child: RepaintBoundary(
                      child: ShaderMask(
                        blendMode: BlendMode.dstIn,
                        shaderCallback: (bounds) => const LinearGradient(
                          begin: Alignment.topCenter,
                          end: Alignment.bottomCenter,
                          colors: [Colors.white, Colors.transparent],
                          stops: [0.05, 0.7],
                        ).createShader(bounds),
                        child: Opacity(
                          opacity: .55,
                          child: ImageFiltered(
                            imageFilter: ImageFilter.blur(
                              sigmaX: 40,
                              sigmaY: 40,
                            ),
                            child: Transform.scale(
                              scale: 2,
                              child: AvatarImageContent(
                                imageUrl: poster,
                                fallback: const SizedBox.shrink(),
                              ),
                            ),
                          ),
                        ),
                      ),
                    ),
                  ),
                ),
              ),
            child,
          ],
        ),
      ),
    );
  }
}
