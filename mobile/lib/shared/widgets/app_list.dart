import 'package:flutter/material.dart';

import '../theme/theme.dart';

/// A flush, borderless settings/list row: leading icon, title, optional
/// subtitle and trailing widget. No card, no background — groups are
/// separated by [AppListSection] dividers instead.
class AppListRow extends StatelessWidget {
  const AppListRow({
    super.key,
    this.icon,
    required this.title,
    this.subtitle,
    this.subtitleStyle,
    this.subtitleMaxLines,
    this.trailing,
    this.titleColor,
    this.onTap,
  });

  final IconData? icon;
  final String title;
  final String? subtitle;
  final TextStyle? subtitleStyle;
  final int? subtitleMaxLines;
  final Widget? trailing;
  final Color? titleColor;
  final VoidCallback? onTap;

  @override
  Widget build(BuildContext context) {
    final row = Padding(
      padding: const EdgeInsets.symmetric(
        horizontal: Grid.gutter,
        vertical: Grid.twelve,
      ),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          if (icon != null) ...[
            Padding(
              padding: const EdgeInsets.only(top: 1),
              child: Icon(
                icon,
                size: 22,
                color: titleColor ?? context.colors.onSurfaceVariant,
              ),
            ),
            const SizedBox(width: Grid.xs),
          ],
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              mainAxisSize: MainAxisSize.min,
              children: [
                Text(
                  title,
                  style: context.textTheme.bodyLarge?.copyWith(
                    color: titleColor,
                  ),
                ),
                if (subtitle != null) ...[
                  const SizedBox(height: Grid.quarter),
                  Text(
                    subtitle!,
                    style:
                        subtitleStyle ??
                        context.textTheme.bodySmall?.copyWith(
                          color: context.colors.onSurfaceVariant,
                        ),
                    maxLines: subtitleMaxLines,
                    overflow: subtitleMaxLines == null
                        ? null
                        : TextOverflow.ellipsis,
                  ),
                ],
              ],
            ),
          ),
          if (trailing != null) ...[const SizedBox(width: Grid.xxs), trailing!],
        ],
      ),
    );

    if (onTap == null) return row;
    return InkWell(onTap: onTap, child: row);
  }
}

/// A custom leading widget variant of [AppListRow] for rows whose leading
/// slot is not a plain [IconData] (e.g. an emoji or image).
class AppListRowRaw extends StatelessWidget {
  const AppListRowRaw({
    super.key,
    required this.leading,
    required this.title,
    this.subtitle,
    this.trailing,
    this.onTap,
  });

  final Widget leading;
  final Widget title;
  final Widget? subtitle;
  final Widget? trailing;
  final VoidCallback? onTap;

  @override
  Widget build(BuildContext context) {
    final row = Padding(
      padding: const EdgeInsets.symmetric(
        horizontal: Grid.gutter,
        vertical: Grid.twelve,
      ),
      child: Row(
        children: [
          leading,
          const SizedBox(width: Grid.xs),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              mainAxisSize: MainAxisSize.min,
              children: [
                title,
                if (subtitle != null) ...[
                  const SizedBox(height: Grid.quarter),
                  subtitle!,
                ],
              ],
            ),
          ),
          if (trailing != null) ...[const SizedBox(width: Grid.xxs), trailing!],
        ],
      ),
    );

    if (onTap == null) return row;
    return InkWell(onTap: onTap, child: row);
  }
}

/// A group of list rows separated from the previous group by a hairline
/// divider with breathing room, Slack-style. An optional [label] renders a
/// small muted header above the rows.
class AppListSection extends StatelessWidget {
  const AppListSection({
    super.key,
    this.label,
    required this.children,
    this.showDivider = true,
  });

  final String? label;
  final List<Widget> children;
  final bool showDivider;

  @override
  Widget build(BuildContext context) {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        if (showDivider)
          Padding(
            padding: const EdgeInsets.symmetric(vertical: Grid.xxs),
            child: Divider(height: 1, color: context.colors.outlineVariant),
          ),
        if (label != null)
          Padding(
            padding: const EdgeInsets.fromLTRB(
              Grid.gutter,
              Grid.xxs,
              Grid.gutter,
              Grid.quarter,
            ),
            child: Text(
              label!.toUpperCase(),
              style: context.textTheme.labelMedium?.copyWith(
                color: context.colors.onSurfaceVariant,
                fontWeight: FontWeight.w600,
                letterSpacing: 0.6,
              ),
            ),
          ),
        ...children,
      ],
    );
  }
}
