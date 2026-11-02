import 'dart:async';
import 'dart:math' show sin, pi;

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/theme/buzz_icons.dart';
import '../../shared/theme/theme.dart';
import 'thread_detail_page.dart';
import 'timeline_message.dart';

/// Reveals the existing reply action as a message is dragged to the right.
class MessageSwipeReply extends HookConsumerWidget {
  final TimelineMessage message;
  final List<TimelineMessage>? allMessages;
  final String channelId;
  final String? currentPubkey;
  final bool isMember;
  final bool isArchived;
  final Widget child;

  /// The visible content, excluding author metadata and reply/reaction footers.
  final GlobalKey contentKey;

  const MessageSwipeReply({
    super.key,
    required this.message,
    required this.allMessages,
    required this.channelId,
    required this.currentPubkey,
    required this.isMember,
    required this.isArchived,
    required this.child,
    required this.contentKey,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final offset = useAnimationController(upperBound: double.infinity);
    final pulse = useAnimationController(
      duration: const Duration(milliseconds: 180),
    );
    final dragDistance = useRef(0.0);
    final layoutKey = useMemoized(GlobalKey.new);
    final contentCenter = useState<double?>(null);
    final didHaptic = useRef(false);
    final cancelled = useRef(false);
    final reducedMotion = MediaQuery.disableAnimationsOf(context);
    final messages = allMessages;
    if (messages == null || message.isSystem) return child;

    void reset() {
      pulse.stop();
      pulse.value = 0;
      if (reducedMotion) {
        offset.value = 0;
      } else {
        unawaited(
          offset.animateWith(
            ScrollSpringSimulation(
              const SpringDescription(mass: 1, stiffness: 500, damping: 36),
              offset.value,
              0,
              0,
            ),
          ),
        );
      }
    }

    // Flutter ends an accepted drag on PointerCancel too. Preserve the raw
    // cancellation signal so an interrupted gesture cannot open a reply.
    return Listener(
      onPointerCancel: (_) => cancelled.value = true,
      child: GestureDetector(
        behavior: HitTestBehavior.translucent,
        excludeFromSemantics: true,
        onHorizontalDragStart: (_) {
          final contentBox = contentKey.currentContext?.findRenderObject();
          final layoutBox = layoutKey.currentContext?.findRenderObject();
          if (contentBox is RenderBox && layoutBox is RenderBox) {
            contentCenter.value = layoutBox
                .globalToLocal(
                  contentBox.localToGlobal(contentBox.size.center(Offset.zero)),
                )
                .dy;
          }
          offset.stop();
          pulse.stop();
          pulse.value = 0;
          offset.value = 0;
          dragDistance.value = 0;
          didHaptic.value = false;
          cancelled.value = false;
        },
        onHorizontalDragUpdate: (details) {
          dragDistance.value = (dragDistance.value + details.delta.dx).clamp(
            0.0,
            double.infinity,
          );
          // Keep pulling beyond readiness, with increasing rubber-band resistance.
          final excess = dragDistance.value - 72;
          offset.value = excess <= 0
              ? dragDistance.value
              : 72 + 40 * excess / (excess + 80);
          if (offset.value >= 72 && !didHaptic.value) {
            didHaptic.value = true;
            unawaited(HapticFeedback.mediumImpact());
            if (!reducedMotion) unawaited(pulse.forward(from: 0));
          }
          if (offset.value < 72) {
            didHaptic.value = false;
            pulse.stop();
            pulse.value = 0;
          }
        },
        onHorizontalDragCancel: reset,
        onHorizontalDragEnd: (_) {
          final reply = !cancelled.value && offset.value >= 72;
          reset();
          if (!reply) return;
          Navigator.of(context).push(
            MaterialPageRoute<void>(
              builder: (_) => ThreadDetailPage(
                threadHead: message,
                allMessages: messages,
                channelId: channelId,
                currentPubkey: currentPubkey,
                isMember: isMember,
                isArchived: isArchived,
              ),
            ),
          );
        },
        child: AnimatedBuilder(
          animation: Listenable.merge([offset, pulse]),
          child: child,
          builder: (context, child) {
            final progress = (offset.value / 72).clamp(0.0, 1.0);
            // Follows the chosen message color, like the user's own bubbles.
            final chatColors = ChatColors.of(context);
            final accent = chatColors.outgoing;
            final filledForeground = chatColors.foreground;
            return Stack(
              key: layoutKey,
              clipBehavior: Clip.none,
              alignment: Alignment.centerLeft,
              children: [
                if (progress > 0)
                  Positioned(
                    left: 0,
                    top: contentCenter.value ?? 0,
                    width: Grid.lg,
                    child: FractionalTranslation(
                      translation: const Offset(0, -0.5),
                      child: ExcludeSemantics(
                        child: Opacity(
                          opacity: progress,
                          child: Transform.scale(
                            scale: reducedMotion
                                ? 1
                                : (0.5 + 0.5 * progress) *
                                      (1 + 0.12 * sin(pi * pulse.value)),
                            alignment: Alignment.center,
                            child: SizedBox(
                              key: const ValueKey('swipe-reply-feedback'),
                              width: Grid.lg,
                              child: Column(
                                mainAxisSize: MainAxisSize.min,
                                children: [
                                  SizedBox.square(
                                    dimension: Grid.lg,
                                    child: DecoratedBox(
                                      key: const ValueKey('swipe-reply-circle'),
                                      decoration: BoxDecoration(
                                        shape: BoxShape.circle,
                                        color: progress == 1
                                            ? accent
                                            : Colors.transparent,
                                      ),
                                      child: Stack(
                                        fit: StackFit.expand,
                                        children: [
                                          CircularProgressIndicator(
                                            value: progress,
                                            strokeWidth: 2,
                                            strokeAlign: -1,
                                            color: accent,
                                            backgroundColor: Colors.transparent,
                                          ),
                                          Icon(
                                            BuzzIcons.messageSquareReply,
                                            size: 22,
                                            color: progress == 1
                                                ? filledForeground
                                                : context.colors.primary,
                                          ),
                                        ],
                                      ),
                                    ),
                                  ),
                                  const SizedBox(height: Grid.quarter),
                                  Text(
                                    'Reply',
                                    textAlign: TextAlign.center,
                                    style: context.textTheme.labelSmall
                                        ?.copyWith(
                                          color:
                                              context.colors.onSurfaceVariant,
                                        ),
                                  ),
                                ],
                              ),
                            ),
                          ),
                        ),
                      ),
                    ),
                  ),
                Transform.translate(
                  offset: Offset(offset.value, 0),
                  child: child,
                ),
              ],
            );
          },
        ),
      ),
    );
  }
}
