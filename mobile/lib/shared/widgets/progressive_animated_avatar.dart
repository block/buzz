import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

import '../animated_avatar.dart';
import '../relay/media_image.dart';
import 'avatar_image.dart';

/// Paints an animated avatar's persisted poster until its first frame is ready.
///
/// Callers own playback policy, which keeps animation opt-in at individual
/// profile surfaces.
class ProgressiveAnimatedAvatar extends HookWidget {
  const ProgressiveAnimatedAvatar({
    super.key,
    required this.descriptor,
    required this.fallback,
    this.fit = BoxFit.cover,
  });

  final AnimatedAvatarDescriptor descriptor;
  final Widget fallback;
  final BoxFit fit;

  @override
  Widget build(BuildContext context) {
    final readyAnimationUrl = useState<String?>(null);
    final scheduledAnimationUrl = useRef<String?>(null);
    final animationKey = useMemoized(GlobalKey.new, [descriptor.animationUrl]);
    final isReady = readyAnimationUrl.value == descriptor.animationUrl;
    final animation = KeyedSubtree(
      key: const ValueKey('progressive-animated-avatar-animation'),
      child: MediaImage(
        key: animationKey,
        url: descriptor.animationUrl,
        fit: fit,
        errorBuilder: (_, _, _) => const SizedBox.shrink(),
        frameBuilder: (context, child, frame, wasSynchronouslyLoaded) {
          if ((wasSynchronouslyLoaded || frame != null) &&
              !isReady &&
              scheduledAnimationUrl.value != descriptor.animationUrl) {
            scheduledAnimationUrl.value = descriptor.animationUrl;
            WidgetsBinding.instance.addPostFrameCallback((_) {
              if (context.mounted) {
                readyAnimationUrl.value = descriptor.animationUrl;
              }
            });
          }
          return child;
        },
      ),
    );

    if (isReady) {
      return KeyedSubtree(
        key: const ValueKey('progressive-animated-avatar-animation-ready'),
        child: animation,
      );
    }

    return Stack(
      fit: StackFit.expand,
      children: [
        AvatarImageContent(
          key: const ValueKey('progressive-animated-avatar-poster'),
          imageUrl: descriptor.posterUrl,
          fallback: fallback,
          fit: fit,
        ),
        Offstage(
          key: const ValueKey('progressive-animated-avatar-animation-loading'),
          offstage: true,
          child: animation,
        ),
      ],
    );
  }
}
