import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

import '../../shared/agents/agent_avatar_catalog.dart';
import '../../shared/animated_avatar.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/ios_glass_navigation_button.dart';
import 'agent_avatar_category.dart';
import 'agent_character_artwork.dart';

/// Reserves a full scaled label line below each category control.
double agentAvatarChoicesHeight(BuildContext context) {
  final label = TextPainter(
    text: TextSpan(
      text: 'Buzzies',
      style: context.textTheme.labelSmall?.copyWith(fontSize: 12),
    ),
    textScaler: MediaQuery.textScalerOf(context),
    textDirection: Directionality.of(context),
    maxLines: 1,
  )..layout();
  final height = 68 + Grid.half + label.height.ceilToDouble();
  label.dispose();
  return height < 92 ? 92 : height;
}

/// Character packs and existing avatar editors in one horizontal choice rail.
class AgentAvatarChoices extends HookWidget {
  const AgentAvatarChoices({
    super.key,
    required this.category,
    required this.examples,
    required this.emoji,
    required this.disabled,
    required this.onPack,
    required this.onEmoji,
    required this.onImage,
  });

  final AgentAvatarCategory category;
  final Map<AgentAvatarPack, int> examples;
  final String emoji;
  final bool disabled;
  final ValueChanged<AgentAvatarPack> onPack;
  final VoidCallback onEmoji;
  final VoidCallback onImage;

  @override
  Widget build(BuildContext context) {
    final controller = useScrollController();
    useEffect(() {
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (!controller.hasClients) return;
        final position = category.position;
        controller.jumpTo(
          (position * 84.0 - 84).clamp(
            0.0,
            controller.position.maxScrollExtent,
          ),
        );
      });
      return null;
    }, const []);
    return SizedBox(
      height: agentAvatarChoicesHeight(context),
      child: SingleChildScrollView(
        controller: controller,
        scrollDirection: Axis.horizontal,
        clipBehavior: Clip.none,
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            for (final item in AgentAvatarPack.values)
              _Choice(
                label: item.label,
                selected: category.pack == item,
                onTap: disabled ? null : () => onPack(item),
                child: AgentCharacterAlignment(
                  url: agentAvatarCatalog[examples[item]!].url,
                  scale: 0.85,
                  child: Transform.scale(
                    scale: animatedAvatarArtworkScale,
                    child: characterPoster(
                      agentAvatarCatalog[examples[item]!].url,
                    ),
                  ),
                ),
              ),
            _Choice(
              label: 'Emoji',
              selected: category == AgentAvatarCategory.emoji,
              onTap: disabled ? null : onEmoji,
              child: Center(
                child: Text(emoji, style: const TextStyle(fontSize: 32)),
              ),
            ),
            _Choice(
              label: 'Image',
              selected: category == AgentAvatarCategory.image,
              onTap: disabled ? null : onImage,
              child: const Icon(Icons.image_outlined, size: 28),
            ),
          ],
        ),
      ),
    );
  }
}

class _Choice extends StatelessWidget {
  const _Choice({
    required this.label,
    required this.selected,
    required this.onTap,
    required this.child,
  });

  final String label;
  final bool selected;
  final VoidCallback? onTap;
  final Widget child;

  @override
  Widget build(BuildContext context) {
    void activate() {
      if (!selected) HapticFeedback.selectionClick();
      onTap?.call();
    }

    if (defaultTargetPlatform == TargetPlatform.iOS) {
      return Padding(
        padding: const EdgeInsets.only(right: Grid.half),
        child: SizedBox(
          width: 80,
          child: Column(
            children: [
              IosGlassNavigationButton(
                key: ValueKey('agent-avatar-choice-$label'),
                icon: IosGlassNavigationIcon.emoji,
                label: '',
                semanticLabel: label,
                onPressed: onTap == null ? null : activate,
                width: 68,
                height: 68,
                controlSize: 60,
                isSelected: selected,
                tintsSelection: false,
                artwork: child,
              ),
              const SizedBox(height: Grid.half),
              ExcludeSemantics(
                child: Text(
                  label,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: context.textTheme.labelSmall?.copyWith(
                    fontSize: 12,
                    color: selected
                        ? context.colors.primary
                        : context.colors.onSurfaceVariant.withValues(
                            alpha: 0.9,
                          ),
                    fontWeight: selected ? FontWeight.w600 : FontWeight.w500,
                  ),
                ),
              ),
            ],
          ),
        ),
      );
    }
    return Semantics(
      label: label,
      button: true,
      selected: selected,
      enabled: onTap != null,
      child: Padding(
        padding: EdgeInsets.zero,
        child: InkWell(
          key: ValueKey('agent-avatar-choice-$label'),
          onTap: onTap == null ? null : activate,
          borderRadius: BorderRadius.circular(Radii.container),
          child: Padding(
            padding: const EdgeInsets.symmetric(horizontal: Grid.xxs),
            child: ExcludeSemantics(
              child: Column(
                children: [
                  Container(
                    width: 68,
                    height: 68,
                    padding: const EdgeInsets.all(3),
                    decoration: BoxDecoration(
                      shape: BoxShape.circle,
                      border: Border.all(color: Colors.transparent, width: 2),
                    ),
                    child: ClipOval(
                      child: ColoredBox(
                        color: context.colors.surfaceContainerHighest,
                        child: child,
                      ),
                    ),
                  ),
                  const SizedBox(height: Grid.half),
                  Text(
                    label,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: context.textTheme.labelSmall?.copyWith(
                      fontSize: 12,
                      color: selected
                          ? context.colors.primary
                          : context.colors.onSurfaceVariant.withValues(
                              alpha: 0.9,
                            ),
                      fontWeight: selected ? FontWeight.w600 : FontWeight.w500,
                    ),
                  ),
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }
}
