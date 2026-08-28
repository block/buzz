part of '../theme_picker_page.dart';

/// A compact preview container holding complete Home and Chat device frames.
///
/// Each 393 x 852 frame uses `contain` within the compact outer card, keeping
/// both complete devices in view with breathing room instead of filling it.
class _ThemeHomePreview extends StatelessWidget {
  const _ThemeHomePreview({
    required this.colorScheme,
    required this.topSectionGradient,
  });

  static const deviceSize = Size(393, 852);

  final ColorScheme colorScheme;
  final Gradient? topSectionGradient;

  @override
  Widget build(BuildContext context) {
    final previewTheme = colorScheme.brightness == Brightness.dark
        ? AppTheme.dark(
            colorScheme: colorScheme,
            topSectionGradient: topSectionGradient,
          )
        : AppTheme.light(
            colorScheme: colorScheme,
            topSectionGradient: topSectionGradient,
          );

    return Container(
      key: const ValueKey('theme-home-preview'),
      padding: const EdgeInsets.symmetric(vertical: Grid.twelve),
      decoration: BoxDecoration(
        color: context.colors.surfaceContainerLow,
        borderRadius: BorderRadius.circular(Radii.container),
      ),
      clipBehavior: Clip.antiAlias,
      child: FittedBox(
        fit: BoxFit.contain,
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            _PreviewDeviceFrame(
              key: const ValueKey('theme-home-device-preview'),
              theme: previewTheme,
              semanticLabel: 'Home preview',
              child: const _FigmaHomeScreen(),
            ),
            const SizedBox(width: 96),
            _PreviewDeviceFrame(
              key: const ValueKey('theme-chat-device-preview'),
              theme: previewTheme,
              semanticLabel: 'Chat preview',
              child: const _FigmaChatScreen(),
            ),
          ],
        ),
      ),
    );
  }
}

class _PreviewDeviceFrame extends StatelessWidget {
  const _PreviewDeviceFrame({
    required this.theme,
    required this.semanticLabel,
    required this.child,
    super.key,
  });

  final ThemeData theme;
  final String semanticLabel;
  final Widget child;

  @override
  Widget build(BuildContext context) => Semantics(
    container: true,
    excludeSemantics: true,
    label: semanticLabel,
    image: true,
    child: Padding(
      padding: const EdgeInsets.all(2),
      child: Container(
        width: _ThemeHomePreview.deviceSize.width,
        height: _ThemeHomePreview.deviceSize.height,
        decoration: BoxDecoration(
          borderRadius: BorderRadius.circular(40),
          border: Border.all(
            color: theme.colorScheme.outline.withValues(alpha: 0.72),
            width: 2,
            strokeAlign: BorderSide.strokeAlignOutside,
          ),
        ),
        clipBehavior: Clip.none,
        child: ClipRRect(
          borderRadius: BorderRadius.circular(40),
          clipBehavior: Clip.antiAlias,
          child: Theme(data: theme, child: child),
        ),
      ),
    ),
  );
}

class _FigmaHomeScreen extends ConsumerWidget {
  const _FigmaHomeScreen();

  static const _rowTops = <double>[
    116,
    152,
    188,
    224,
    266,
    302,
    338,
    374,
    416,
    452,
    488,
    524,
  ];
  static const _capsuleWidths = <double>[
    69,
    69,
    107,
    91,
    69,
    69,
    107,
    91,
    69,
    69,
    107,
    91,
  ];

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final scheme = context.colors;
    final community = ref.watch(activeCommunityProvider).value;
    final communityName = community?.name.trim();
    final relayUrl = community?.relayUrl;
    final communityIcon = relayUrl == null
        ? null
        : ref.watch(communityIconProvider(relayUrl)).value;
    final suppliedGradient = context.appColors.topSectionGradient;
    final backgroundGradient =
        suppliedGradient ??
        LinearGradient(
          begin: Alignment.topCenter,
          end: Alignment.bottomCenter,
          colors: [scheme.surfaceContainerLowest, scheme.surface],
        );
    final channelForeground = navigationPrimaryForeground(
      context,
    ).withValues(alpha: 0.38);
    final sectionForeground = navigationSectionForeground(
      context,
    ).withValues(alpha: 0.52);
    final chrome = scheme.surfaceContainerHighest;

    return ColoredBox(
      key: const ValueKey('theme-preview-surface'),
      color: scheme.surface,
      child: DecoratedBox(
        decoration: BoxDecoration(gradient: backgroundGradient),
        child: Stack(
          children: [
            Positioned(
              key: const ValueKey('theme-preview-community-identity'),
              left: 12,
              top: 38,
              child: Row(
                children: [
                  AvatarImage(
                    key: const ValueKey('theme-preview-community-icon'),
                    imageUrl: communityIcon,
                    radius: 20,
                    backgroundColor: scheme.primaryContainer,
                    fallback: Text(
                      communityName == null || communityName.isEmpty
                          ? '?'
                          : communityName.substring(0, 1).toUpperCase(),
                      style: context.textTheme.labelMedium?.copyWith(
                        color: scheme.onPrimaryContainer,
                        fontWeight: FontWeight.w600,
                      ),
                    ),
                  ),
                  const SizedBox(width: Grid.twelve),
                  ConstrainedBox(
                    constraints: const BoxConstraints(maxWidth: 210),
                    child: Text(
                      communityName == null || communityName.isEmpty
                          ? 'Community'
                          : communityName,
                      key: const ValueKey('theme-preview-community-name'),
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: context.textTheme.titleMedium?.copyWith(
                        color: navigationPrimaryForeground(context),
                        fontWeight: FontWeight.w600,
                      ),
                    ),
                  ),
                ],
              ),
            ),
            for (var index = 0; index < _rowTops.length; index++)
              Positioned(
                left: index % 4 == 0 ? 22 : 38,
                top: _rowTops[index],
                child: _PreviewChannelSkeletonRow(
                  key: index % 4 == 0
                      ? ValueKey('theme-preview-section-${index ~/ 4}')
                      : ValueKey('theme-preview-channel-$index'),
                  capsuleKey: index % 4 == 0
                      ? ValueKey('theme-preview-section-${index ~/ 4}-capsule')
                      : ValueKey('theme-preview-channel-$index-capsule'),
                  color: index % 4 == 0 ? sectionForeground : channelForeground,
                  capsuleWidth: _capsuleWidths[index],
                  gap: index % 4 == 0 ? 10 : 8,
                ),
              ),
            _PreviewPill(
              key: const ValueKey('theme-preview-bottom-navigation'),
              left: 93,
              top: 767,
              width: 204,
              height: 54,
              color: chrome,
            ),
            _PreviewPill(
              key: const ValueKey('theme-preview-accent-action'),
              left: 317,
              top: 767,
              width: 54,
              height: 54,
              color: scheme.primary,
            ),
          ],
        ),
      ),
    );
  }
}

class _PreviewChannelSkeletonRow extends StatelessWidget {
  const _PreviewChannelSkeletonRow({
    required this.color,
    required this.capsuleWidth,
    required this.gap,
    this.capsuleKey,
    super.key,
  });

  final Color color;
  final double capsuleWidth;
  final double gap;
  final Key? capsuleKey;

  @override
  Widget build(BuildContext context) => Row(
    children: [
      Container(
        width: 12,
        height: 12,
        decoration: BoxDecoration(color: color, shape: BoxShape.circle),
      ),
      SizedBox(width: gap),
      Container(
        key: capsuleKey,
        width: capsuleWidth,
        height: 11,
        decoration: BoxDecoration(
          color: color,
          borderRadius: BorderRadius.circular(Radii.full),
        ),
      ),
    ],
  );
}

class _FigmaChatScreen extends StatelessWidget {
  const _FigmaChatScreen();

  @override
  Widget build(BuildContext context) {
    final scheme = context.colors;
    final chrome = scheme.surfaceContainerHighest;
    final text = scheme.onSurface.withValues(alpha: 0.9);
    final muted = scheme.onSurfaceVariant.withValues(alpha: 0.58);

    return ColoredBox(
      key: const ValueKey('theme-chat-preview-surface'),
      color: scheme.surface,
      child: Stack(
        children: [
          _PreviewPill(left: 16, top: 28, width: 38, height: 38, color: chrome),
          _PreviewPill(
            key: const ValueKey('theme-chat-header-title'),
            left: 70,
            top: 31,
            width: 112,
            height: 13,
            color: text,
          ),
          _PreviewPill(
            key: const ValueKey('theme-chat-header-members'),
            left: 70,
            top: 51,
            width: 66,
            height: 9,
            color: muted,
          ),
          _PreviewPill(
            left: 339,
            top: 28,
            width: 38,
            height: 38,
            color: chrome,
          ),
          _PreviewPill(
            left: 165,
            top: 112,
            width: 64,
            height: 18,
            color: chrome,
          ),
          _PreviewTimelineMessage(
            key: const ValueKey('theme-chat-timeline-message-1'),
            top: 166,
            avatarColor: chrome,
            authorWidth: 92,
            bodyWidths: const [218, 164],
          ),
          _PreviewTimelineMessage(
            key: const ValueKey('theme-chat-timeline-message-2'),
            top: 288,
            avatarColor: chrome,
            authorWidth: 118,
            bodyWidths: const [244, 198, 126],
          ),
          _PreviewTimelineMessage(
            key: const ValueKey('theme-chat-timeline-message-3'),
            top: 432,
            avatarColor: chrome,
            authorWidth: 76,
            bodyWidths: const [206, 148],
          ),
          Positioned(
            key: const ValueKey('theme-chat-composer-preview'),
            left: 16,
            right: 16,
            bottom: 30,
            height: 52,
            child: DecoratedBox(
              decoration: BoxDecoration(
                color: scheme.surfaceContainerHighest,
                borderRadius: BorderRadius.circular(Radii.dialog),
                border: Border.all(color: Colors.black.withValues(alpha: 0.04)),
              ),
              child: Stack(
                children: [
                  _PreviewPill(
                    left: 20,
                    top: 20,
                    width: 104,
                    height: 12,
                    color: muted,
                  ),
                  Positioned(
                    key: const ValueKey('theme-chat-send-action'),
                    right: Grid.xxs,
                    top: 8,
                    width: 36,
                    height: 36,
                    child: DecoratedBox(
                      decoration: BoxDecoration(
                        color: scheme.primary,
                        shape: BoxShape.circle,
                      ),
                    ),
                  ),
                ],
              ),
            ),
          ),
        ],
      ),
    );
  }
}

class _PreviewTimelineMessage extends StatelessWidget {
  const _PreviewTimelineMessage({
    required this.top,
    required this.avatarColor,
    required this.authorWidth,
    required this.bodyWidths,
    super.key,
  });

  final double top;
  final Color avatarColor;
  final double authorWidth;
  final List<double> bodyWidths;

  @override
  Widget build(BuildContext context) {
    final scheme = context.colors;
    return Positioned(
      left: 18,
      right: 18,
      top: top,
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Container(
            width: 38,
            height: 38,
            decoration: BoxDecoration(
              color: avatarColor,
              shape: BoxShape.circle,
            ),
          ),
          const SizedBox(width: 12),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Row(
                  children: [
                    Container(
                      width: authorWidth,
                      height: 13,
                      decoration: BoxDecoration(
                        color: scheme.onSurface.withValues(alpha: 0.9),
                        borderRadius: BorderRadius.circular(8),
                      ),
                    ),
                    const SizedBox(width: 10),
                    Container(
                      width: 42,
                      height: 9,
                      decoration: BoxDecoration(
                        color: scheme.onSurfaceVariant.withValues(alpha: 0.58),
                        borderRadius: BorderRadius.circular(8),
                      ),
                    ),
                  ],
                ),
                const SizedBox(height: 12),
                for (final width in bodyWidths) ...[
                  Container(
                    width: width,
                    height: 11,
                    decoration: BoxDecoration(
                      color: scheme.onSurface.withValues(alpha: 0.88),
                      borderRadius: BorderRadius.circular(8),
                    ),
                  ),
                  const SizedBox(height: 9),
                ],
              ],
            ),
          ),
        ],
      ),
    );
  }
}

class _PreviewPill extends StatelessWidget {
  const _PreviewPill({
    required this.left,
    required this.top,
    required this.width,
    required this.height,
    required this.color,
    super.key,
  });

  final double left;
  final double top;
  final double width;
  final double height;
  final Color color;

  @override
  Widget build(BuildContext context) {
    return Positioned(
      left: left,
      top: top,
      width: width,
      height: height,
      child: DecoratedBox(
        decoration: BoxDecoration(
          color: color,
          borderRadius: BorderRadius.circular(80),
        ),
      ),
    );
  }
}
