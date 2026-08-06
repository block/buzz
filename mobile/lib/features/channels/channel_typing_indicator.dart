import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/theme/theme.dart';
import '../../shared/utils/string_utils.dart';
import '../profile/user_cache_provider.dart';
import 'channel_typing_provider.dart';
import 'small_avatar.dart';

/// Composer-adjacent status for people currently typing in a channel or thread.
class ChannelTypingIndicator extends ConsumerWidget {
  final List<TypingEntry> entries;

  const ChannelTypingIndicator({super.key, required this.entries});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final userCache = ref.watch(userCacheProvider);
    final names = entries.map((entry) {
      final profile =
          userCache[entry.pubkey.toLowerCase()] ??
          ref.read(userCacheProvider.notifier).get(entry.pubkey.toLowerCase());
      return profile?.label ?? shortPubkey(entry.pubkey);
    }).toList();
    final text = switch (names.length) {
      1 => '${names[0]} is typing…',
      2 => '${names[0]} and ${names[1]} are typing…',
      _ => '${names[0]} and ${names.length - 1} others are typing…',
    };
    final visibleEntries = entries.take(3).toList();
    final avatarCount = visibleEntries.length;

    return Padding(
      padding: const EdgeInsets.only(
        left: Grid.twelve,
        right: Grid.twelve,
        bottom: Grid.xxs,
      ),
      child: Container(
        key: const ValueKey('channel-typing-indicator'),
        width: double.infinity,
        padding: const EdgeInsets.symmetric(
          horizontal: Grid.xxs,
          vertical: Grid.xxs,
        ),
        decoration: BoxDecoration(
          color: context.colors.surfaceContainerHighest,
          borderRadius: BorderRadius.circular(Radii.dialog),
          border: Border.all(
            color: Colors.black.withValues(alpha: 0.04),
            width: 1,
          ),
        ),
        child: Row(
          children: [
            SizedBox(
              width: 24.0 + (avatarCount - 1) * 14.0,
              height: 24,
              child: Stack(
                children: [
                  for (var i = 0; i < avatarCount; i++)
                    Positioned(
                      left: i * 14.0,
                      child: SmallAvatar(
                        pubkey: visibleEntries[i].pubkey,
                        userCache: userCache,
                        size: 24,
                      ),
                    ),
                ],
              ),
            ),
            const SizedBox(width: Grid.xxs),
            Flexible(
              child: _TypingTextShimmer(
                text,
                style: context.textTheme.labelSmall?.copyWith(
                  color: context.colors.onSurfaceVariant,
                ),
              ),
            ),
          ],
        ),
      ),
    );
  }
}

class _TypingTextShimmer extends HookWidget {
  final String text;
  final TextStyle? style;

  const _TypingTextShimmer(this.text, {this.style});

  @override
  Widget build(BuildContext context) {
    final animation = useAnimationController(
      duration: const Duration(milliseconds: 2600),
    );
    final reducedMotion = MediaQuery.disableAnimationsOf(context);
    final baseColor = style?.color ?? context.colors.onSurfaceVariant;
    final highlightColor =
        Color.lerp(context.colors.surface, baseColor, 0.4) ?? baseColor;

    useEffect(() {
      if (reducedMotion) {
        animation
          ..stop()
          ..value = 0;
      } else {
        animation.repeat();
      }
      return animation.stop;
    }, [animation, reducedMotion]);

    final label = Text(text, style: style, overflow: TextOverflow.ellipsis);
    if (reducedMotion) return label;

    return RepaintBoundary(
      child: AnimatedBuilder(
        animation: animation,
        child: label,
        builder: (context, child) {
          final center = 1.5 - (animation.value * 3);
          return ShaderMask(
            key: const ValueKey('channel-typing-shimmer'),
            blendMode: BlendMode.srcIn,
            shaderCallback: (bounds) => LinearGradient(
              begin: Alignment(center - 1, 0),
              end: Alignment(center + 1, 0),
              colors: [
                baseColor,
                baseColor,
                highlightColor,
                baseColor,
                baseColor,
              ],
              stops: const [0, 0.34, 0.5, 0.66, 1],
            ).createShader(bounds),
            child: child,
          );
        },
      ),
    );
  }
}
