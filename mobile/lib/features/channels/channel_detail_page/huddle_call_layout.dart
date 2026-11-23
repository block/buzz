part of '../channel_detail_page.dart';

const _huddleRailControlSize = 48.0;
const _huddleRailControlGap = 12.0;

/// Keeps call actions in the same physical regions as workspace navigation.
class _HuddleCallLayout extends StatelessWidget {
  const _HuddleCallLayout({
    required this.strip,
    required this.header,
    required this.content,
    required this.controls,
  });

  final FoldStrip? strip;
  final Widget header;
  final Widget content;
  final Widget controls;

  @override
  Widget build(BuildContext context) {
    final strip = this.strip;
    if (strip == null) {
      return SafeArea(
        child: Padding(
          padding: const EdgeInsets.fromLTRB(
            Grid.gutter,
            Grid.xxs,
            Grid.gutter,
            Grid.gutter,
          ),
          child: Column(
            children: [
              header,
              Expanded(child: content),
              controls,
            ],
          ),
        ),
      );
    }
    final windowSize = MediaQuery.sizeOf(context);
    final metrics = IosNavigationMetrics.of(context);
    final measuredTop = metrics.windowSize == windowSize
        ? metrics.railTopClearance
        : null;
    // The cover status cluster extends below its reported reserved region.
    // Give it breathing room without shifting the inner display controls.
    final coverStatusGap = windowSize.shortestSide < 600 ? 24.0 : 0.0;
    final top =
        strip.rect.top +
        coverStatusGap +
        (measuredTop ??
            (strip.isClearanceEstimated && windowSize.shortestSide >= 600
                ? duoInnerDisplayEstimatedRailClearance
                : strip.topClearance));
    // Side controls sit clear of the centered home indicator. Match the native
    // call rail's edge inset instead of inheriting the bottom navigation inset.
    final bottom = max(strip.bottomClearance, 24.0);
    return Stack(
      children: [
        Positioned.fill(
          key: const ValueKey('huddle-call-full-width-content'),
          child: SafeArea(
            left: false,
            right: false,
            child: Padding(
              padding: const EdgeInsets.all(Grid.gutter),
              child: content,
            ),
          ),
        ),
        Positioned(
          key: const ValueKey('huddle-call-rail'),
          left:
              metrics.railCenter(windowSize, strip.rect.center.dx) -
              strip.width / 2,
          top: top,
          bottom: bottom,
          width: strip.width,
          child: LayoutBuilder(
            builder: (context, constraints) {
              return SingleChildScrollView(
                child: ConstrainedBox(
                  constraints: BoxConstraints(minHeight: constraints.maxHeight),
                  child: Column(
                    mainAxisAlignment: MainAxisAlignment.spaceBetween,
                    children: [header, controls],
                  ),
                ),
              );
            },
          ),
        ),
      ],
    );
  }
}
