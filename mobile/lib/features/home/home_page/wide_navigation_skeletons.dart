part of '../home_page.dart';

/// Covers the active tablet pane while the relay changes. The left list and
/// right timeline preserve the Inbox/channel spatial structure, so no old
/// community text is visible during the handoff.
class _WideCommunityContentSkeleton extends StatelessWidget {
  const _WideCommunityContentSkeleton({super.key});

  @override
  Widget build(BuildContext context) {
    Widget inboxRow(double nameWidth, double previewWidth) {
      return Padding(
        padding: const EdgeInsets.symmetric(
          horizontal: Grid.gutter,
          vertical: Grid.sm,
        ),
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            SkeletonBar(
              width: 36,
              height: 36,
              borderRadius: BorderRadius.circular(Radii.full),
            ),
            const SizedBox(width: Grid.twelve),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  SkeletonBar(width: nameWidth, height: 14),
                  const SizedBox(height: Grid.half),
                  SkeletonBar(width: previewWidth, height: 12),
                  const SizedBox(height: Grid.half),
                  const SkeletonBar(width: 96, height: 12),
                ],
              ),
            ),
          ],
        ),
      );
    }

    Widget messageRow(int index) {
      const nameWidths = [112.0, 88.0, 128.0, 96.0];
      const lineWidths = [196.0, 164.0, 212.0, 148.0];
      return Padding(
        padding: const EdgeInsets.only(bottom: Grid.lg),
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            SkeletonBar(
              width: 36,
              height: 36,
              borderRadius: BorderRadius.circular(Radii.full),
            ),
            const SizedBox(width: Grid.twelve),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Row(
                    children: [
                      SkeletonBar(width: nameWidths[index], height: 15),
                      const SizedBox(width: Grid.xxs),
                      const SkeletonBar(width: 48, height: 12),
                    ],
                  ),
                  const SizedBox(height: Grid.half),
                  SkeletonBar(width: lineWidths[index], height: 15),
                  const SizedBox(height: Grid.half),
                  SkeletonBar(width: lineWidths[index] * 0.72, height: 15),
                ],
              ),
            ),
          ],
        ),
      );
    }

    return LayoutBuilder(
      builder: (context, constraints) {
        final inboxWidth = (constraints.maxWidth * 0.34)
            .clamp(220.0, 340.0)
            .toDouble();
        return Row(
          children: [
            SizedBox(
              width: inboxWidth,
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  const Padding(
                    padding: EdgeInsets.fromLTRB(
                      Grid.gutter,
                      Grid.lg,
                      Grid.gutter,
                      Grid.md,
                    ),
                    child: SkeletonBar(width: 88, height: 24),
                  ),
                  Divider(height: 1, color: context.colors.outlineVariant),
                  Expanded(
                    child: ListView(
                      physics: const NeverScrollableScrollPhysics(),
                      padding: EdgeInsets.zero,
                      children: [
                        inboxRow(84, 112),
                        inboxRow(112, 120),
                        inboxRow(72, 124),
                        inboxRow(96, 108),
                      ],
                    ),
                  ),
                ],
              ),
            ),
            VerticalDivider(width: 1, color: context.colors.outlineVariant),
            Expanded(
              child: Padding(
                padding: const EdgeInsets.fromLTRB(
                  Grid.gutter,
                  Grid.lg,
                  Grid.gutter,
                  Grid.gutter,
                ),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    const SkeletonBar(width: 176, height: 24),
                    const SizedBox(height: Grid.lg),
                    Expanded(
                      child: ListView(
                        physics: const NeverScrollableScrollPhysics(),
                        padding: EdgeInsets.zero,
                        children: [
                          messageRow(0),
                          messageRow(1),
                          messageRow(2),
                          messageRow(3),
                        ],
                      ),
                    ),
                    const SkeletonBar(width: double.infinity, height: 48),
                  ],
                ),
              ),
            ),
          ],
        );
      },
    );
  }
}
