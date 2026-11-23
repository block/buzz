part of '../thread_detail_page.dart';

double _threadUpLevelRowHeight(BuildContext context) =>
    math.max(44, MediaQuery.textScalerOf(context).scale(14) * 1.25 + Grid.xs);

/// The link pinned under the thread header that names where the thread
/// lives, "In #channel" or "In thread · …", and goes there. A null
/// [onPressed] shows it inert while its target loads.
class _ThreadUpLevelRow extends StatelessWidget {
  final String target;
  final String semanticLabel;
  final VoidCallback? onPressed;

  const _ThreadUpLevelRow({
    required this.target,
    required this.semanticLabel,
    required this.onPressed,
  });

  @override
  Widget build(BuildContext context) {
    final enabled = onPressed != null;
    final muted = context.colors.onSurfaceVariant;
    final style = context.textTheme.labelMedium?.copyWith(color: muted);
    return Semantics(
      container: true,
      button: enabled,
      enabled: enabled,
      label: semanticLabel,
      onTap: onPressed,
      excludeSemantics: true,
      child: Material(
        type: MaterialType.transparency,
        child: InkWell(
          key: const ValueKey('thread-up-level'),
          onTap: onPressed,
          child: DecoratedBox(
            decoration: BoxDecoration(
              border: Border(
                bottom: BorderSide(color: context.colors.outlineVariant),
              ),
            ),
            child: Padding(
              padding: const EdgeInsets.symmetric(horizontal: Grid.gutter),
              child: Row(
                children: [
                  Icon(BuzzIcons.cornerUpLeft, size: 16, color: muted),
                  const SizedBox(width: Grid.xxs),
                  Expanded(
                    child: Text.rich(
                      TextSpan(
                        text: 'In ',
                        children: [
                          TextSpan(
                            text: target,
                            style: enabled
                                ? TextStyle(color: context.colors.primary)
                                : null,
                          ),
                        ],
                      ),
                      key: const ValueKey('thread-up-level-label'),
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: style,
                    ),
                  ),
                  if (!enabled) ...[
                    const SizedBox(width: Grid.xxs),
                    BuzzLoadingIndicator(size: 14, color: muted),
                  ],
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }
}
