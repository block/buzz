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
    this.posterPlaceholder,
  });

  final String imageUrl;
  final double size;
  final Widget fallback;

  /// A preloaded poster for spatial transitions in the character picker.
  final Widget? posterPlaceholder;

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
    final route = ModalRoute.of(context);
    useListenable(isIos ? route?.animation : null);
    useListenable(isIos ? route?.secondaryAnimation : null);
    // UIKit video layers cannot reliably follow Flutter's Hero clipping and
    // route opacity. Keep them out of the composition until navigation settles.
    final routeSettled =
        (route?.animation == null ||
            route!.animation!.status == AnimationStatus.completed) &&
        (route?.secondaryAnimation == null ||
            route!.secondaryAnimation!.status == AnimationStatus.dismissed);
    final showNativeVideo =
        !isIos ||
        (routeSettled &&
            route?.isCurrent != false &&
            TickerMode.valuesOf(context).enabled);
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
    final presented = useState<VideoPlayerController?>(null);
    useListenable(
      isIos && posterPlaceholder != null && presented.value != controller
          ? controller
          : null,
    );
    useEffect(() {
      if (isIos &&
          posterPlaceholder != null &&
          controller != null &&
          ready.value == controller &&
          controller.value.position > Duration.zero) {
        presented.value = controller;
      }
      return null;
    }, [controller, controller?.value.position]);
    final hasPlaybackFrame =
        controller != null && presented.value == controller;
    final lifecycle = useAppLifecycleState();
    final playing =
        showNativeVideo &&
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
                  fallback:
                      posterPlaceholder ??
                      AvatarImageContent(
                        imageUrl: poster,
                        preferThumbnail: true,
                        fallback: fallback,
                      ),
                )
              : showNativeVideo &&
                    controller != null &&
                    ready.value == controller
              ? posterPlaceholder == null || !isIos
                    ? VideoPlayer(controller)
                    : Stack(
                        fit: StackFit.expand,
                        children: [
                          Opacity(
                            opacity: hasPlaybackFrame ? 1 : 0,
                            child: VideoPlayer(controller),
                          ),
                          IgnorePointer(
                            child: AnimatedOpacity(
                              opacity: hasPlaybackFrame ? 0 : 1,
                              duration: const Duration(milliseconds: 160),
                              child: posterPlaceholder!,
                            ),
                          ),
                        ],
                      )
              : posterPlaceholder ??
                    AvatarImageContent(
                      imageUrl: poster,
                      preferThumbnail: true,
                      fallback: fallback,
                    ),
        ),
      ),
    );
  }
}
