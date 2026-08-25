import 'package:flutter/material.dart';

import '../relay/media_image.dart';
import '../animated_avatar.dart';
import 'avatar_image.dart';
import 'progressive_animated_avatar.dart';

/// A circular avatar that opts into playback for animated profile descriptors.
class PlayingAvatarImage extends StatelessWidget {
  /// Creates an avatar that plays animation when the profile URL describes it.
  const PlayingAvatarImage({
    super.key,
    required this.imageUrl,
    required this.radius,
    required this.fallback,
    this.backgroundColor,
    this.loadingImage,
    this.loadingPosterImage,
    this.onAnimationReady,
  });

  /// The still-image URL or encoded animated-avatar descriptor.
  final String? imageUrl;

  /// The radius of the circular avatar.
  final double radius;

  /// Optional background shown behind still-image content.
  final Color? backgroundColor;

  /// A local image shown while the persisted animation warms up.
  final ImageProvider? loadingImage;

  /// A local poster shown when motion is disabled while remote media loads.
  final ImageProvider? loadingPosterImage;

  /// Called after persisted avatar media produces its first frame.
  final VoidCallback? onAnimationReady;

  /// Content shown while media is unavailable or still loading.
  final Widget fallback;

  @override
  Widget build(BuildContext context) {
    final descriptor = parseAnimatedAvatarUrl(imageUrl);
    if (descriptor != null &&
        MediaQuery.disableAnimationsOf(context) &&
        loadingPosterImage != null) {
      return CircleAvatar(
        radius: radius,
        backgroundColor: Colors.transparent,
        child: ClipOval(
          child: SizedBox.square(
            dimension: radius * 2,
            child: Stack(
              fit: StackFit.expand,
              children: [
                Image(
                  image: loadingPosterImage!,
                  fit: BoxFit.cover,
                  gaplessPlayback: true,
                ),
                Offstage(
                  offstage: true,
                  child: MediaImage(
                    url: descriptor.posterUrl,
                    fit: BoxFit.cover,
                    errorBuilder: (_, _, _) => const SizedBox.shrink(),
                    frameBuilder: (_, child, frame, wasSynchronouslyLoaded) {
                      if (wasSynchronouslyLoaded || frame != null) {
                        WidgetsBinding.instance.addPostFrameCallback((_) {
                          onAnimationReady?.call();
                        });
                      }
                      return child;
                    },
                  ),
                ),
              ],
            ),
          ),
        ),
      );
    }
    if (descriptor == null || MediaQuery.disableAnimationsOf(context)) {
      return AvatarImage(
        imageUrl: descriptor?.posterUrl ?? imageUrl,
        radius: radius,
        backgroundColor: backgroundColor,
        fallback: fallback,
      );
    }

    return CircleAvatar(
      radius: radius,
      backgroundColor: Colors.transparent,
      child: ClipOval(
        child: SizedBox.square(
          dimension: radius * 2,
          child: ProgressiveAnimatedAvatar(
            descriptor: descriptor,
            fallback: fallback,
            loadingImage: loadingImage,
            onAnimationReady: onAnimationReady,
          ),
        ),
      ),
    );
  }
}
