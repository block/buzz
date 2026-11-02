import 'agent_category_transition.dart';
import 'dart:math' as math;

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

import '../../shared/agents/agent_avatar_catalog.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/navigation_button_surface.dart';
import 'agent_character_artwork.dart';

/// Centered character artwork and the desktop-style exchanging filmstrip.
class AgentCharacterStage extends HookWidget {
  const AgentCharacterStage({
    super.key,
    required this.pack,
    required this.selected,
    required this.height,
    required this.preview,
    required this.disabled,
    required this.onSelected,
  });

  final AgentAvatarPack pack;
  final int selected;
  final double height;
  final Widget preview;
  final bool disabled;
  final ValueChanged<int> onSelected;

  @override
  Widget build(BuildContext context) {
    final slots = useState(pack.indices.where((i) => i != selected).toList());
    final scroll = useScrollController();
    final rootKey = useMemoized(GlobalKey.new);
    final previewKey = useMemoized(GlobalKey.new);
    final keys = useMemoized(
      () => List.generate(slots.value.length, (_) => GlobalKey()),
    );
    final motion = useAnimationController(
      duration: const Duration(milliseconds: 540),
    );
    useAnimation(motion);
    final flight = useState<_CharacterFlight?>(null);
    final reduceMotion = MediaQuery.disableAnimationsOf(context);
    // Travel for 420 ms, then crossfade the poster over the live surface.
    // A platform view needs a composited frame after becoming visible.
    final travel = (motion.value * 540 / 420).clamp(0.0, 1.0);
    final t = const Cubic(0.77, 0, 0.175, 1).transform(travel);
    final handoff = ((motion.value * 540 - 420) / 120).clamp(0.0, 1.0);
    final flying = flight.value != null && motion.value < 1;

    Rect bounds(GlobalKey key) {
      final root = rootKey.currentContext!.findRenderObject()! as RenderBox;
      final box = key.currentContext!.findRenderObject()! as RenderBox;
      return MatrixUtils.transformRect(
        box.getTransformTo(root),
        Offset.zero & box.size,
      );
    }

    void choose(int slot) {
      if (disabled || flying) return;
      HapticFeedback.selectionClick();
      final incoming = slots.value[slot];
      if (!reduceMotion) {
        flight.value = _CharacterFlight(
          from: bounds(keys[slot]),
          to: Rect.fromCenter(
            center: bounds(previewKey).center,
            width: math.min(440, height - 115),
            height: math.min(440, height - 115),
          ),
          outgoing: selected,
          slot: slot,
        );
        motion.forward(from: 0);
      } else {
        flight.value = null;
        motion.value = 1;
      }
      final next = [...slots.value];
      next[slot] = selected;
      slots.value = next;
      onSelected(incoming);
    }

    final current = flight.value;
    final incomingRect = flying ? _arc(current!.from, current.to, t) : null;
    return SizedBox(
      key: rootKey,
      height: height,
      child: Stack(
        clipBehavior: Clip.none,
        children: [
          Positioned.fill(
            bottom: 115,
            child: AgentCategoryMotion(
              child: Center(
                child: SizedBox.square(
                  key: previewKey,
                  dimension: math.min(440, height - 115),
                  child: OverflowBox(
                    maxWidth: math.min(440, height - 115),
                    maxHeight: math.min(440, height - 115),
                    child: Opacity(
                      opacity: flying && travel < 1 ? 0 : 1,
                      child: preview,
                    ),
                  ),
                ),
              ),
            ),
          ),
          Positioned(
            left: 0,
            right: 0,
            bottom: Grid.sm,
            height: 68,
            child: AgentCategoryMotion(
              slide: true,
              child: NavigationButtonSurface(
                capsule: true,
                child: LayoutBuilder(
                  builder: (context, constraints) => ListView.builder(
                    controller: scroll,
                    key: const ValueKey('agent-avatar-filmstrip'),
                    scrollDirection: Axis.horizontal,
                    padding: const EdgeInsets.symmetric(horizontal: Grid.xxs),
                    itemExtent: 72,
                    itemCount: slots.value.length,
                    itemBuilder: (context, slot) {
                      final i = slots.value[slot];
                      return Semantics(
                        label: agentAvatarCatalog[i].name,
                        button: true,
                        child: InkWell(
                          key: ValueKey('agent-avatar-$i'),
                          onTap: disabled ? null : () => choose(slot),
                          borderRadius: BorderRadius.circular(Radii.container),
                          child: AnimatedBuilder(
                            animation: scroll,
                            builder: (context, child) {
                              final center =
                                  Grid.xxs +
                                  slot * 72 +
                                  36 -
                                  (scroll.hasClients ? scroll.offset : 0);
                              final edgeDistance = math.min(
                                center +
                                    math.max(
                                      0.0,
                                      72 -
                                          (scroll.hasClients
                                              ? scroll.offset
                                              : 0),
                                    ),
                                constraints.maxWidth -
                                    center +
                                    math.max(
                                      0.0,
                                      72 -
                                          (math.max(
                                                0.0,
                                                slots.value.length * 72 +
                                                    Grid.xxs * 2 -
                                                    constraints.maxWidth,
                                              ) -
                                              (scroll.hasClients
                                                  ? scroll.offset
                                                  : 0)),
                                    ),
                              );
                              final progress = (edgeDistance / 72).clamp(
                                0.0,
                                1.0,
                              );
                              final scale = reduceMotion
                                  ? 1.0
                                  : 0.65 +
                                        0.35 *
                                            Curves.easeInOut.transform(
                                              progress,
                                            );
                              return Transform.scale(
                                scale: scale,
                                child: child,
                              );
                            },
                            child: Padding(
                              padding: const EdgeInsets.all(Grid.xxs),
                              child: Opacity(
                                opacity:
                                    flying &&
                                        travel < 1 &&
                                        current!.slot == slot
                                    ? 0
                                    : 1,
                                child: SizedBox(
                                  key: keys[slot],
                                  child: AgentCharacterAlignment(
                                    url: agentAvatarCatalog[i].url,
                                    child: characterPoster(
                                      agentAvatarCatalog[i].url,
                                    ),
                                  ),
                                ),
                              ),
                            ),
                          ),
                        ),
                      );
                    },
                  ),
                ),
              ),
            ),
          ),
          if (flying)
            Positioned.fromRect(
              rect: incomingRect!,
              child: IgnorePointer(
                child: ExcludeSemantics(
                  child: Opacity(
                    opacity: 1 - handoff,
                    child: Transform.scale(
                      scale: 1 - 0.15 * t,
                      child: AgentCharacterAlignment(
                        url: agentAvatarCatalog[selected].url,
                        child: characterPoster(
                          agentAvatarCatalog[selected].url,
                        ),
                      ),
                    ),
                  ),
                ),
              ),
            ),
          if (flying && travel < 1)
            Positioned.fromRect(
              rect: _arc(current!.to, current.from, t),
              child: IgnorePointer(
                child: ExcludeSemantics(
                  child: Transform.scale(
                    scale: 0.85 + 0.15 * t,
                    child: AgentCharacterAlignment(
                      url: agentAvatarCatalog[current.outgoing].url,
                      child: characterPoster(
                        agentAvatarCatalog[current.outgoing].url,
                      ),
                    ),
                  ),
                ),
              ),
            ),
        ],
      ),
    );
  }
}

class _CharacterFlight {
  const _CharacterFlight({
    required this.from,
    required this.to,
    required this.outgoing,
    required this.slot,
  });
  final Rect from;
  final Rect to;
  final int outgoing;
  final int slot;
}

Rect _arc(Rect from, Rect to, double t) {
  final delta = to.center - from.center;
  final length = math.max(1.0, delta.distance);
  final bend = math.sin(t * math.pi) * math.min(90, length * 0.35);
  final center =
      Offset.lerp(from.center, to.center, t)! +
      Offset(-delta.dy / length, delta.dx / length) * bend;
  return Rect.fromCenter(
    center: center,
    width: from.width + (to.width - from.width) * t,
    height: from.height + (to.height - from.height) * t,
  );
}
