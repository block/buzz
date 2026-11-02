import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:video_player/video_player.dart';

import '../animated_avatar.dart';
import 'avatar_image.dart';
import 'avatar_shape.dart';
import 'android_transparent_avatar.dart';

/// Plays the transparent video paired with a bundled character poster.
class PresetAgentAvatar extends HookWidget {
  const PresetAgentAvatar({
    super.key,
    required this.imageUrl,
    required this.size,
    required this.fallback,
  });

  final String imageUrl;
  final double size;
  final Widget fallback;

  @override
  Widget build(BuildContext context) {
    final poster = parseAnimatedAvatarUrl(imageUrl)?.posterUrl ?? imageUrl;
    final uri = Uri.tryParse(poster);
    final canPlay =
        RegExp(
          r'^/avatars/[A-Za-z0-9_-]+/poster/[A-Za-z0-9_-]+/[A-Za-z0-9_-]+\.png$',
        ).hasMatch(uri?.path ?? '') &&
        uri?.host == 'dwwgwmfqqjotj.cloudfront.net' &&
        !MediaQuery.disableAnimationsOf(context);
    final isIos = defaultTargetPlatform == TargetPlatform.iOS;
    final isAndroid = defaultTargetPlatform == TargetPlatform.android;
    final videoUrl = canPlay
        ? uri!.replace(
            path: uri.path
                .replaceFirst('/poster/', isIos ? '/hevc/' : '/webm/')
                .replaceFirst(RegExp(r'\.png$'), isIos ? '.mp4' : '.webm'),
          )
        : null;
    final controller = useMemoized(
      () => videoUrl == null || isAndroid
          ? null
          : VideoPlayerController.networkUrl(
              videoUrl,
              // Let AVPlayerLayer composite HEVC alpha directly on iOS.
              viewType: isIos
                  ? VideoViewType.platformView
                  : VideoViewType.textureView,
              videoPlayerOptions: VideoPlayerOptions(mixWithOthers: true),
            ),
      [videoUrl, isAndroid],
    );
    final ready = useState<VideoPlayerController?>(null);
    final lifecycle = useAppLifecycleState();
    final playing =
        TickerMode.valuesOf(context).enabled &&
        (lifecycle == null || lifecycle == AppLifecycleState.resumed);
    useEffect(() {
      if (controller == null) return null;
      var active = true;
      Future<void> initialize() async {
        try {
          await controller.initialize();
          if (!active) return;
          await controller.setVolume(0);
          await controller.setLooping(true);
          if (!active) return;
          ready.value = controller;
        } catch (_) {
          // Keep the poster if the video is unavailable or unsupported.
        }
      }

      unawaited(initialize());
      return () {
        active = false;
        unawaited(controller.dispose().catchError((Object _) {}));
      };
    }, [controller]);
    useEffect(() {
      if (controller != null && ready.value == controller) {
        unawaited(
          (playing ? controller.play() : controller.pause()).catchError((
            Object _,
          ) {
            if (context.mounted && ready.value == controller) {
              ready.value = null;
            }
          }),
        );
      }
      return null;
    }, [controller, ready.value, playing]);

    return SizedBox.square(
      dimension: size,
      child: AvatarClip(
        isAgent: true,
        child: Transform.scale(
          scale: animatedAvatarArtworkScale,
          child: isAndroid && videoUrl != null
              ? AndroidTransparentAvatar(
                  key: ValueKey(videoUrl),
                  url: videoUrl,
                  playing: playing,
                  fallback: AvatarImageContent(
                    imageUrl: poster,
                    preferThumbnail: true,
                    fallback: fallback,
                  ),
                )
              : controller != null && ready.value == controller
              ? VideoPlayer(controller)
              : AvatarImageContent(
                  imageUrl: poster,
                  preferThumbnail: true,
                  fallback: fallback,
                ),
        ),
      ),
    );
  }
}
