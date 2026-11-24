part of '../thread_detail_page.dart';

/// Height of the two-line Flutter title, so the bar can grow with large text.
double _threadUpLevelTitleHeight(BuildContext context) {
  final scaler = MediaQuery.textScalerOf(context);
  double line(TextStyle? style) =>
      scaler.scale(style?.fontSize ?? 14) * (style?.height ?? 1);
  return line(context.textTheme.titleSmall) + line(context.textTheme.bodySmall);
}

/// Layout gap between the title's icon and the name. The glyph's own margin
/// adds as much again, so what shows is a little over a word space: the same
/// clear space for the text size as the iOS capsule's
/// `NavigationTitleView.subtitleIconGap` leaves around its smaller text.
const _threadUpLevelIconGap = 3.0;

/// The thread title wherever Flutter draws it: "Thread" over the place the
/// thread lives in, which the whole title goes to. iOS shows the same in its
/// native title capsule.
class _ThreadUpLevelTitle extends StatelessWidget {
  const _ThreadUpLevelTitle(this.up);

  final _ThreadUpLevel up;

  @override
  Widget build(BuildContext context) {
    final enabled = up.onPressed != null;
    final muted = context.colors.onSurfaceVariant;
    final iconSize = MediaQuery.textScalerOf(context).scale(14);
    final icon = up.icon;
    return Align(
      alignment: Alignment.centerLeft,
      child: Semantics(
        container: true,
        button: enabled,
        enabled: enabled,
        label: up.semanticLabel,
        onTap: up.onPressed,
        excludeSemantics: true,
        child: Material(
          type: MaterialType.transparency,
          child: InkWell(
            key: const ValueKey('thread-up-level'),
            borderRadius: BorderRadius.circular(Radii.md),
            onTap: up.onPressed,
            child: ConstrainedBox(
              constraints: const BoxConstraints(minHeight: 48),
              child: Padding(
                padding: const EdgeInsets.symmetric(horizontal: Grid.xxs),
                child: Column(
                  mainAxisSize: MainAxisSize.min,
                  mainAxisAlignment: MainAxisAlignment.center,
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(
                      'Thread',
                      key: const ValueKey('thread-app-bar-title'),
                      style: context.textTheme.titleSmall?.copyWith(
                        fontWeight: FontWeight.w600,
                      ),
                    ),
                    Row(
                      mainAxisSize: MainAxisSize.min,
                      children: [
                        if (icon != null) ...[
                          Icon(
                            icon,
                            key: const ValueKey('thread-up-level-icon'),
                            size: iconSize,
                            color: muted,
                          ),
                          // A prefix of the name, as a typed "#" would be.
                          const SizedBox(width: _threadUpLevelIconGap),
                        ],
                        Flexible(
                          child: Text(
                            up.label,
                            key: const ValueKey('thread-up-level-label'),
                            maxLines: 1,
                            overflow: TextOverflow.ellipsis,
                            style: context.textTheme.bodySmall?.copyWith(
                              color: muted,
                            ),
                          ),
                        ),
                        const SizedBox(width: Grid.quarter),
                        if (enabled)
                          Icon(
                            BuzzIcons.chevronRight,
                            key: const ValueKey('thread-up-level-chevron'),
                            size: iconSize,
                            color: muted,
                          )
                        else
                          BuzzLoadingIndicator(
                            key: const ValueKey('thread-up-level-loading'),
                            size: iconSize - 2,
                            color: muted,
                          ),
                      ],
                    ),
                  ],
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }
}
