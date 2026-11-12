part of '../compose_bar.dart';

class _UploadProgressMotion extends StatelessWidget {
  final bool visible;
  final double progress;
  final bool reducedMotion;
  final VoidCallback onCancel;

  const _UploadProgressMotion({
    required this.visible,
    required this.progress,
    required this.reducedMotion,
    required this.onCancel,
  });

  @override
  Widget build(BuildContext context) {
    return AnimatedSwitcher(
      key: const ValueKey('compose-upload-progress-motion'),
      duration: reducedMotion
          ? Duration.zero
          : const Duration(milliseconds: 220),
      reverseDuration: reducedMotion
          ? Duration.zero
          : const Duration(milliseconds: 180),
      transitionBuilder: (child, animation) {
        final motion = CurvedAnimation(
          parent: animation,
          curve: Curves.easeInCubic,
          reverseCurve: Curves.easeOutCubic,
        );
        return ClipRect(
          child: SlideTransition(
            position: Tween<Offset>(
              begin: const Offset(0, 1.15),
              end: Offset.zero,
            ).animate(motion),
            child: child,
          ),
        );
      },
      child: visible
          ? Padding(
              key: const ValueKey('compose-upload-progress-visible'),
              padding: const EdgeInsets.only(bottom: Grid.xxs),
              child: _UploadProgressPill(
                progress: progress,
                reducedMotion: reducedMotion,
                onCancel: onCancel,
              ),
            )
          : const SizedBox.shrink(
              key: ValueKey('compose-upload-progress-hidden'),
            ),
    );
  }
}

/// A lightweight post-send upload status that stays above the composer, so the
/// composer is immediately available for the next message.
class _UploadProgressPill extends HookConsumerWidget {
  final double progress;
  final bool reducedMotion;
  final VoidCallback onCancel;

  const _UploadProgressPill({
    required this.progress,
    required this.reducedMotion,
    required this.onCancel,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final cancelHovered = useState(false);
    final clampedProgress = progress.clamp(0.0, 1.0).toDouble();
    final percentage = (clampedProgress * 100).round();
    return Semantics(
      liveRegion: true,
      label: 'Uploading $percentage%',
      child: SizedBox(
        key: const ValueKey('compose-upload-progress'),
        width: 300,
        height: 36,
        child: ClipRRect(
          borderRadius: BorderRadius.circular(Radii.full),
          child: DecoratedBox(
            decoration: BoxDecoration(
              color: context.colors.surfaceContainerHighest,
              border: Border.all(color: context.colors.outlineVariant),
              borderRadius: BorderRadius.circular(Radii.full),
            ),
            child: Stack(
              fit: StackFit.expand,
              children: [
                Align(
                  alignment: Alignment.centerLeft,
                  child: AnimatedFractionallySizedBox(
                    key: const ValueKey('compose-upload-progress-fill'),
                    duration: reducedMotion
                        ? Duration.zero
                        : const Duration(milliseconds: 150),
                    curve: Curves.easeOutCubic,
                    widthFactor: clampedProgress,
                    heightFactor: 1,
                    child: ColoredBox(
                      color: context.colors.primary.withValues(
                        alpha: context.theme.brightness == Brightness.dark
                            ? 0.15
                            : 0.12,
                      ),
                    ),
                  ),
                ),
                Padding(
                  padding: const EdgeInsets.only(
                    left: Grid.twelve,
                    right: Grid.half,
                  ),
                  child: Row(
                    children: [
                      Expanded(
                        child: Text(
                          'Uploading',
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis,
                          style: context.textTheme.labelMedium?.copyWith(
                            color: context.colors.onSurface,
                            fontWeight: FontWeight.w600,
                          ),
                        ),
                      ),
                      SizedBox(
                        width: 36,
                        child: FittedBox(
                          alignment: Alignment.centerRight,
                          fit: BoxFit.scaleDown,
                          child: Text(
                            '$percentage%',
                            style: context.textTheme.labelMedium?.copyWith(
                              color: context.colors.onSurfaceVariant,
                              fontWeight: FontWeight.w600,
                            ),
                          ),
                        ),
                      ),
                      const SizedBox(width: Grid.xxs),
                      Padding(
                        padding: const EdgeInsets.symmetric(
                          vertical: Grid.half,
                        ),
                        child: MouseRegion(
                          onEnter: (_) => cancelHovered.value = true,
                          onExit: (_) => cancelHovered.value = false,
                          child: Material(
                            color: cancelHovered.value
                                ? context.colors.onSurface.withValues(
                                    alpha: 0.08,
                                  )
                                : Colors.transparent,
                            borderRadius: BorderRadius.circular(Radii.full),
                            clipBehavior: Clip.antiAlias,
                            child: InkWell(
                              key: const ValueKey('compose-upload-cancel'),
                              onTap: onCancel,
                              borderRadius: BorderRadius.circular(Radii.full),
                              child: Padding(
                                key: const ValueKey(
                                  'compose-upload-cancel-padding',
                                ),
                                padding: const EdgeInsets.symmetric(
                                  horizontal: Grid.half,
                                  vertical: Grid.quarter,
                                ),
                                child: Text(
                                  'Cancel',
                                  style: context.textTheme.labelMedium
                                      ?.copyWith(
                                        color: context.colors.onSurface,
                                        fontWeight: FontWeight.w600,
                                      ),
                                ),
                              ),
                            ),
                          ),
                        ),
                      ),
                    ],
                  ),
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}
