import 'dart:ui';
import 'package:flutter/material.dart';
import '../animated_avatar.dart';
import '../theme/theme.dart';
import 'avatar_image.dart';

const profilePortraitSize = 80.0;

/// A desktop-style profile container with static, faded portrait artwork.
class AgentArtworkPanel extends StatelessWidget {
  const AgentArtworkPanel({
    required this.imageUrl,
    required this.child,
    this.borderRadius,
    this.personProfile = false,
    super.key,
  });
  final String? imageUrl;
  final bool personProfile;
  final Widget child;
  final BorderRadius? borderRadius;

  @override
  Widget build(BuildContext context) {
    final poster = parseAnimatedAvatarUrl(imageUrl)?.posterUrl ?? imageUrl;
    final artwork = AvatarImageContent(
      imageUrl: poster,
      preferThumbnail: true,
      fallback: const SizedBox.shrink(),
    );
    return ClipRRect(
      borderRadius: borderRadius ?? BorderRadius.circular(Radii.card),
      child: ColoredBox(
        color: Theme.of(context).scaffoldBackgroundColor,
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
                          stops: [0.05, 1.0],
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
                              child: FittedBox(
                                fit: BoxFit.cover,
                                child: SizedBox.square(
                                  // Match the portrait's decoded-image cache key.
                                  dimension: profilePortraitSize,
                                  child: artwork,
                                ),
                              ),
                            ),
                          ),
                        ),
                      ),
                    ),
                  ),
                ),
              ),
            if (personProfile)
              SizedBox(width: double.infinity, child: child)
            else
              child,
          ],
        ),
      ),
    );
  }
}
