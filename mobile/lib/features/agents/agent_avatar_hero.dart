import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

import '../../shared/animated_avatar.dart';
import '../../shared/widgets/avatar_shape.dart';
import 'agent_avatar_picker.dart';

/// Keeps native playback mounted while a cached image carries the route motion.
class AgentAvatarHero extends HookWidget {
  const AgentAvatarHero({
    super.key,
    required this.tag,
    required this.selection,
    required this.child,
    this.verticalOffset = 0,
  });

  final Object? tag;
  final AgentAvatarSelection selection;
  final Widget child;
  final double verticalOffset;

  @override
  Widget build(BuildContext context) {
    final liveKey = useMemoized(GlobalKey.new);
    if (tag == null || MediaQuery.disableAnimationsOf(context)) return child;
    final route = ModalRoute.of(context);
    final animations = [
      if (route?.animation != null) route!.animation!,
      if (route?.secondaryAnimation != null) route!.secondaryAnimation!,
    ];
    final endpoint = _AvatarEndpoint(
      selection: selection,
      verticalOffset: verticalOffset,
      child: KeyedSubtree(
        key: liveKey,
        child: AnimatedBuilder(
          animation: Listenable.merge(animations),
          builder: (context, live) {
            // iOS presets already substitute their poster during navigation.
            // Do not fade a UIKit video layer under a second Flutter opacity.
            if (defaultTargetPlatform == TargetPlatform.iOS) return live!;
            final moving = animations.any((animation) => animation.isAnimating);
            return Stack(
              fit: StackFit.passthrough,
              children: [
                AnimatedOpacity(
                  opacity: moving ? 0 : 1,
                  duration: moving
                      ? Duration.zero
                      : const Duration(milliseconds: 160),
                  child: live!,
                ),
                Positioned.fill(
                  child: IgnorePointer(
                    child: AnimatedOpacity(
                      opacity: moving ? 1 : 0,
                      duration: moving
                          ? Duration.zero
                          : const Duration(milliseconds: 160),
                      child: _poster(selection, verticalOffset),
                    ),
                  ),
                ),
              ],
            );
          },
          child: child,
        ),
      ),
    );
    return Hero(
      tag: tag!,
      // Reparent the same keyed live subtree instead of disposing its player.
      placeholderBuilder: (_, size, child) => SizedBox.fromSize(
        size: size,
        child: Offstage(child: TickerMode(enabled: false, child: child)),
      ),
      flightShuttleBuilder: (_, animation, direction, from, to) {
        final source = (from.widget as Hero).child as _AvatarEndpoint;
        return AnimatedBuilder(
          animation: animation,
          builder: (_, _) {
            // The destination rebuilds with the saved selection after pop.
            // Cancel retains its original selection throughout the flight.
            final target = (to.widget as Hero).child as _AvatarEndpoint;
            final t = direction == HeroFlightDirection.push
                ? animation.value
                : 1 - animation.value;
            return _poster(
              target.selection,
              source.verticalOffset +
                  (target.verticalOffset - source.verticalOffset) * t,
            );
          },
        );
      },
      child: endpoint,
    );
  }
}

Widget _poster(AgentAvatarSelection selection, double offset) =>
    FractionalTranslation(
      translation: Offset(0, offset),
      child: AvatarClip(
        isAgent: true,
        child: selection.bytes != null
            ? Image.memory(
                selection.bytes!,
                fit: BoxFit.cover,
                gaplessPlayback: true,
              )
            : Transform.scale(
                scale: animatedAvatarArtworkScale,
                child: Image.network(
                  selection.url!,
                  fit: BoxFit.contain,
                  gaplessPlayback: true,
                ),
              ),
      ),
    );

class _AvatarEndpoint extends StatelessWidget {
  const _AvatarEndpoint({
    required this.selection,
    required this.verticalOffset,
    required this.child,
  });
  final AgentAvatarSelection selection;
  final double verticalOffset;
  final Widget child;
  @override
  Widget build(BuildContext context) => child;
}
