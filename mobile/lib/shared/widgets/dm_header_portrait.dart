import 'package:flutter/material.dart';

import '../animated_avatar.dart';
import 'avatar_image.dart';
import 'avatar_shape.dart';
import 'preset_agent_avatar.dart';
import 'progressive_animated_avatar.dart';

/// A decorative DM portrait, with transparent playback for animated agents.
class DmHeaderPortrait extends StatelessWidget {
  const DmHeaderPortrait({
    super.key,
    required this.imageUrl,
    required this.isAgent,
    required this.initial,
    required this.size,
    this.standalone = false,
  });

  final String? imageUrl;
  final bool isAgent;
  final String initial;
  final double size;
  final bool standalone;

  static bool playsAnimation(String? url, bool isAgent) =>
      isAgent && usesInsetAgentArtwork(url);

  @override
  Widget build(BuildContext context) {
    final portrait = _buildPortrait(context);
    return isAgent && !standalone
        ? Transform.scale(scale: 0.9, child: portrait)
        : portrait;
  }

  Widget _buildPortrait(BuildContext context) {
    final colors = Theme.of(context).colorScheme;
    final textStyle = Theme.of(context).textTheme.titleSmall;
    final fallback = Center(
      child: Text(
        initial,
        style: textStyle?.copyWith(
          color: colors.onPrimaryContainer,
          fontSize: (textStyle.fontSize ?? 14) * (size / 48).clamp(0.0, 1.0),
        ),
      ),
    );
    if (!playsAnimation(imageUrl, isAgent)) {
      return AvatarImage(
        imageUrl: imageUrl,
        isAgent: isAgent,
        radius: size / 2,
        backgroundColor: colors.primaryContainer,
        fallback: fallback,
      );
    }
    final descriptor = parseAnimatedAvatarUrl(imageUrl);
    final poster = descriptor?.posterUrl ?? imageUrl!;
    final Widget artwork;
    if (usesInsetAgentArtwork(poster)) {
      artwork = PresetAgentAvatar(
        imageUrl: imageUrl!,
        size: size,
        fallback: fallback,
      );
    } else {
      artwork = SizedBox.square(
        dimension: size,
        child: AvatarClip(
          isAgent: true,
          child: Transform.scale(
            scale: animatedAvatarArtworkScale,
            child: MediaQuery.disableAnimationsOf(context)
                ? AvatarImageContent(imageUrl: poster, fallback: fallback)
                : ProgressiveAnimatedAvatar(
                    descriptor: descriptor!,
                    fallback: fallback,
                  ),
          ),
        ),
      );
    }
    // Transparent characters have inset artwork; align their visible silhouette
    // with the navigation controls while keeping the title and cluster fixed.
    return Transform.translate(
      offset: Offset(0, standalone ? -6 : 0),
      child: artwork,
    );
  }
}
