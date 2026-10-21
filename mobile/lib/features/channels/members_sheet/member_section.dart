part of '../members_sheet.dart';

/// The member card is split into independently laid-out rows so a large group
/// cannot force every offscreen member to mount during the sheet entrance.
class _MemberSection extends StatelessWidget {
  const _MemberSection({
    required this.label,
    required this.itemCount,
    required this.itemBuilder,
    this.semanticIndexOffset = 0,
  });

  final String label;
  final int itemCount;
  final IndexedWidgetBuilder itemBuilder;
  final int semanticIndexOffset;

  @override
  Widget build(BuildContext context) => SliverPadding(
    padding: const EdgeInsets.symmetric(vertical: Grid.xxs),
    sliver: SliverMainAxisGroup(
      slivers: [
        SliverToBoxAdapter(
          child: Padding(
            padding: const EdgeInsets.only(left: Grid.half, bottom: Grid.xxs),
            child: Text(
              label,
              style: context.textTheme.labelMedium?.copyWith(
                color: context.colors.onSurfaceVariant,
                fontWeight: FontWeight.w600,
              ),
            ),
          ),
        ),
        SliverList(
          delegate: SliverChildBuilderDelegate(
            (context, index) => Material(
              color: context.colors.surfaceContainerHighest,
              borderRadius: BorderRadius.vertical(
                top: index == 0
                    ? const Radius.circular(Radii.container)
                    : Radius.zero,
                bottom: index == itemCount - 1
                    ? const Radius.circular(Radii.container)
                    : Radius.zero,
              ),
              clipBehavior: Clip.antiAlias,
              child: ListTileTheme.merge(
                contentPadding: const EdgeInsets.symmetric(horizontal: Grid.xs),
                minLeadingWidth: 22,
                horizontalTitleGap: Grid.xs,
                child: Column(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    if (index > 0)
                      Divider(
                        height: 1,
                        thickness: 1,
                        indent: Grid.xs + 40 + Grid.xs,
                        endIndent: Grid.xs,
                        color: context.colors.onSurface.withValues(alpha: 0.07),
                      ),
                    itemBuilder(context, index),
                  ],
                ),
              ),
            ),
            childCount: itemCount,
            semanticIndexOffset: semanticIndexOffset,
          ),
        ),
      ],
    ),
  );
}
