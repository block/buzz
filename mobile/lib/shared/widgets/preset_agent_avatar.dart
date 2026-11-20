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
    final videoShown =
        showNativeVideo && controller != null && ready.value == controller;
    // A fresh AVPlayerLayer draws nothing until its first frame arrives, so
    // keep the poster under it until playback advances in this showing.
    final shownFrom = useState<(Duration, Duration)?>(null);
    final presented = useState(false);
    useListenable(isIos && videoShown && !presented.value ? controller : null);
    useEffect(() {
      if (!videoShown) {
        presented.value = false;
      } else if (shownFrom.value case (final fresh, final polled)
          // A failed player resets its position to zero and stops polling.
          when !controller.value.hasError &&
              controller.value.position != fresh &&
              controller.value.position != polled) {
        presented.value = true;
      }
      return null;
    }, [videoShown, controller, shownFrom.value, controller?.value.position]);
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
    final playback = useRef<Future<void>>(Future.value());
    useEffect(() {
      if (controller != null && ready.value == controller) {
        playback.value = (playing ? controller.play() : controller.pause())
            .catchError((Object _) {
              if (context.mounted && ready.value == controller) {
                ready.value = null;
              }
            });
      }
      return null;
    }, [controller, ready.value, playing]);
    // The baseline is read from the player once the play command lands,
    // since play() seeks a finished loop back to zero. The controller's last
    // polled position is kept beside it: while paused that value lags the
    // player, so it differs from the baseline until a poll in this showing.
    useEffect(() {
      shownFrom.value = null;
      // While paused (an inactive app), wait for the play() that resumes it.
      if (!isIos || !videoShown || !playing || presented.value) return null;
      var active = true;
      unawaited(
        playback.value
            .then((_) => controller.position)
            .then((position) {
              if (active) {
                shownFrom.value = (
                  position ?? Duration.zero,
                  controller.value.position,
                );
              }
            })
            .catchError((Object _) {}),
      );
      return () => active = false;
    }, [videoShown, playing, controller]);

    final nativeVideoVisible =
        isIos &&
        showNativeVideo &&
        controller != null &&
        ready.value == controller;
    // HEVC artwork carries its own alpha silhouette. A Flutter path clip around
    // its platform view can also clip selection overlays above other native views.
    Widget clipArtwork(Widget child) =>
        nativeVideoVisible ? child : AvatarClip(isAgent: true, child: child);

    // Keyed by URL so a new character never shows the previous one's poster
    // while its own loads.
    final posterImage = KeyedSubtree(
      key: ValueKey(poster),
      child:
          posterPlaceholder ??
          AvatarImageContent(
            imageUrl: poster,
            preferThumbnail: true,
            fallback: fallback,
          ),
    );
    return SizedBox.square(
      dimension: size,
      child: clipArtwork(
        Transform.scale(
          scale: animatedAvatarArtworkScale,
          child: isAndroid && videoUrl != null
              ? AndroidTransparentAvatar(
                  key: ValueKey(videoUrl),
                  url: videoUrl,
                  playing: playing,
                  fallback: posterImage,
                )
              : !isIos
              ? videoShown
                    ? VideoPlayer(controller)
                    : posterImage
              : Stack(
                  fit: StackFit.expand,
                  children: [
                    // The poster stays below the native layer, in Flutter's
                    // base layer: an overlay above it costs a blank frame.
                    // It stays mounted under the video so the next showing
                    // covers at once instead of reloading it.
                    IgnorePointer(
                      key: const ValueKey('preset-poster'),
                      child: AnimatedOpacity(
                        opacity: videoShown && presented.value ? 0 : 1,
                        // Cover a hidden player at once; fade only onto
                        // video.
                        duration: videoShown && presented.value
                            ? const Duration(milliseconds: 160)
                            : Duration.zero,
                        child: posterImage,
                      ),
                    ),
                    if (videoShown)
                      Opacity(
                        key: const ValueKey('preset-video'),
                        opacity: presented.value ? 1 : 0,
                        child: VideoPlayer(controller),
                      ),
                  ],
                ),
        ),
      ),
    );
  }
}
