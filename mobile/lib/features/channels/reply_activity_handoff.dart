import 'dart:math' as math;

import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

import '../../shared/profile/user_profile.dart';
import '../../shared/theme/theme.dart';
import 'small_avatar.dart';
import 'typing_dots.dart';

/// Keeps one avatar mounted as its working pill becomes a reply summary.
class ReplyActivityHandoff extends HookWidget {
  const ReplyActivityHandoff({
    super.key,
    required this.pubkey,
    required this.userCache,
    required this.working,
    required this.label,
  });

  final String pubkey;
  final Map<String, UserProfile> userCache;
  final bool working;
  final String? label;

  @override
  Widget build(BuildContext context) {
    final knownAgent = useRef(false);
    if (working) knownAgent.value = true;
    final existingLabel = useRef(false);
    if (working) existingLabel.value = label != null;
    final reducedMotion = MediaQuery.disableAnimationsOf(context);
    final controller = useAnimationController(
      duration: const Duration(milliseconds: 200),
      initialValue: working ? 0 : 1,
    );
    useEffect(() {
      final target = working ? 0.0 : 1.0;
      if (reducedMotion) {
        controller.value = target;
      } else {
        controller.animateTo(target, curve: Curves.easeInOutCubic);
      }
      return null;
    }, [working, reducedMotion]);
    final animated = useAnimation(controller);
    final progress = reducedMotion ? (working ? 0.0 : 1.0) : animated;
    final style = context.textTheme.labelMedium?.copyWith(
      color: context.colors.onSurfaceVariant,
    );
    return LayoutBuilder(
      builder: (context, constraints) {
        final text = TextPainter(
          text: TextSpan(text: label ?? '', style: style),
          textDirection: Directionality.of(context),
          textScaler: MediaQuery.textScalerOf(context),
          maxLines: 2,
          ellipsis: '…',
        )..layout(maxWidth: math.max(0, constraints.maxWidth - 24));
        final textWidth = text.width;
        final height = math.max(
          constraints.minHeight,
          math.max(28.0, text.height),
        );
        text.dispose();
        // 20px avatar, 4px gap, 19px dots, and 6/8px container insets.
        final pendingWidth = existingLabel.value ? 61 + textWidth : 57.0;
        final width = pendingWidth + (24 + textWidth - pendingWidth) * progress;
        return SizedBox(
          width: width,
          height: height,
          child: Stack(
            clipBehavior: Clip.none,
            children: [
              Positioned(
                left: 0,
                top: (height - 28) / 2,
                height: 28,
                width: 57 + (width - 57) * progress,
                child: DecoratedBox(
                  key: const ValueKey('thread-working-container'),
                  decoration: BoxDecoration(
                    color: context.controlSurface.withValues(
                      alpha: 1 - progress,
                    ),
                    borderRadius: BorderRadius.circular(Grid.twelve),
                  ),
                ),
              ),
              Positioned(
                left: 6 * (1 - progress),
                top: (height - 20) / 2,
                child: SmallAvatar(
                  pubkey: pubkey,
                  userCache: userCache,
                  agent: knownAgent.value,
                ),
              ),
              if (progress < 1)
                Positioned(
                  left: 30 - 6 * progress,
                  top: (height - 5) / 2,
                  child: Opacity(
                    opacity: 1 - progress,
                    child: const TypingDots(
                      key: ValueKey('thread-working-dots'),
                      dotSize: 5,
                    ),
                  ),
                ),
              if (label != null)
                Positioned(
                  left: existingLabel.value ? 61 - 37 * progress : 24,
                  right: 0,
                  top: 0,
                  bottom: 0,
                  child: ClipRect(
                    child: OverflowBox(
                      alignment: Alignment.centerRight,
                      minHeight: 0,
                      maxHeight: height,
                      minWidth: textWidth,
                      maxWidth: textWidth,
                      child: Opacity(
                        opacity: working || existingLabel.value ? 1 : progress,
                        child: Transform.scale(
                          scale: working || existingLabel.value
                              ? 1
                              : 0.95 + 0.05 * progress,
                          alignment: Alignment.centerRight,
                          child: Text(
                            label!,
                            style: style,
                            maxLines: 2,
                            overflow: TextOverflow.ellipsis,
                          ),
                        ),
                      ),
                    ),
                  ),
                ),
            ],
          ),
        );
      },
    );
  }
}
