import 'package:flutter/material.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../relay/relay_provider.dart';
import '../theme/theme.dart';
import 'sticker_reference.dart';

const double stickerPreviewSize = 200;
const double compactStickerPreviewSize = 64;

/// Renders a Sonar sticker exclusively through the active relay's verified
/// cache endpoint.
class StickerPreview extends ConsumerWidget {
  final StickerTagParseResult stickerTag;
  final double size;

  const StickerPreview({
    super.key,
    required this.stickerTag,
    this.size = stickerPreviewSize,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final reference = stickerTag.reference;
    if (stickerTag.status != StickerTagStatus.valid || reference == null) {
      return const StickerUnavailablePlaceholder();
    }

    final relayBaseUrl = ref.watch(relayConfigProvider).baseUrl;
    final url = reference.cacheUrl(relayBaseUrl);
    if (url == null) return const StickerUnavailablePlaceholder();

    return Semantics(
      image: true,
      label: 'Sticker :${reference.shortcode}:',
      child: ConstrainedBox(
        constraints: BoxConstraints(maxWidth: size, maxHeight: size),
        child: Image.network(
          url,
          key: ValueKey('message-sticker-image:$url'),
          width: size,
          height: size,
          fit: BoxFit.contain,
          filterQuality: FilterQuality.medium,
          semanticLabel: 'Sticker :${reference.shortcode}:',
          errorBuilder: (_, _, _) => const StickerUnavailablePlaceholder(),
        ),
      ),
    );
  }
}

/// Stable failure state for malformed, unresolvable, or failed sticker loads.
class StickerUnavailablePlaceholder extends StatelessWidget {
  const StickerUnavailablePlaceholder({super.key});

  @override
  Widget build(BuildContext context) {
    return Container(
      key: const ValueKey('message-sticker-unavailable'),
      constraints: const BoxConstraints(maxWidth: stickerPreviewSize),
      padding: const EdgeInsets.symmetric(
        horizontal: Grid.sm,
        vertical: Grid.xs,
      ),
      decoration: BoxDecoration(
        color: context.colors.surfaceContainerHighest,
        borderRadius: BorderRadius.circular(Radii.md),
        border: Border.all(color: context.colors.outlineVariant),
      ),
      child: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          Icon(
            LucideIcons.imageOff,
            size: 18,
            color: context.colors.onSurfaceVariant,
          ),
          const SizedBox(width: Grid.xxs),
          Flexible(
            child: Text(
              'Sticker unavailable',
              style: context.textTheme.labelMedium?.copyWith(
                color: context.colors.onSurfaceVariant,
              ),
            ),
          ),
        ],
      ),
    );
  }
}
