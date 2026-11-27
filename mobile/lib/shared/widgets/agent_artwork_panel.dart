import 'dart:ui';
import 'package:flutter/material.dart';
import '../animated_avatar.dart';
import '../theme/theme.dart';
import 'avatar_image.dart';

const profilePortraitSize = 80.0;
const teamPortraitSize = 108.0;

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
    final sampledPortrait =
        personProfile && Theme.of(context).brightness == Brightness.dark;
    final extendsThroughHeader =
        personProfile && Theme.of(context).platform == TargetPlatform.iOS;
    final baseBlur = sampledPortrait ? 18.0 : 40.0;
    final blur = extendsThroughHeader ? baseBlur * 1.56 : baseBlur;
    final boundedArtwork = sampledPortrait
        ? ColorFiltered(
            colorFilter: const ColorFilter.mode(
              Color(0xffb0b0b0),
              BlendMode.modulate,
            ),
            child: artwork,
          )
        : artwork;
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
                        shaderCallback: (bounds) => LinearGradient(
                          begin: Alignment.topCenter,
                          end: Alignment.bottomCenter,
                          colors: const [Colors.white, Colors.transparent],
                          stops: [extendsThroughHeader ? 0.75 : 0.05, 1.0],
                        ).createShader(bounds),
                        child: Opacity(
                          opacity:
                              personProfile &&
                                  Theme.of(context).brightness ==
                                      Brightness.light
                              ? .30
                              : .55,
                          child: ImageFiltered(
                            imageFilter: ImageFilter.blur(
                              sigmaX: blur,
                              sigmaY: blur,
                            ),
                            child: Transform.scale(
                              scale: sampledPortrait ? 1.25 : 2,
                              child: FittedBox(
                                fit: BoxFit.cover,
                                child: SizedBox.square(
                                  // Match the portrait's decoded-image cache key.
                                  dimension: profilePortraitSize,
                                  child: boundedArtwork,
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
