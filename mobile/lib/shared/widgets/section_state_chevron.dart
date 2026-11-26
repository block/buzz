import 'dart:async';

import 'package:flutter/material.dart';
import '../theme/buzz_icons.dart';
import '../theme/theme.dart';

class SectionHeadingLabel extends StatelessWidget {
  const SectionHeadingLabel({
    super.key,
    required this.label,
    required this.expanded,
    this.icon,
  });
  final bool expanded;
  final String label;
  final Widget? icon;

  @override
  Widget build(BuildContext context) => Row(
    children: [
      ?icon,
      Flexible(
        child: Text(
          label,
          maxLines: 1,
          overflow: TextOverflow.ellipsis,
          style: navigationBucketHeadingStyle(context),
        ),
      ),
      SectionStateChevron(key: ValueKey(label), expanded: expanded),
    ],
  );
}

/// Briefly indicates the new disclosure state without shifting the title.
class SectionStateChevron extends StatefulWidget {
  const SectionStateChevron({super.key, required this.expanded});

  final bool expanded;

  @override
  State<SectionStateChevron> createState() => _SectionStateChevronState();
}

class _SectionStateChevronState extends State<SectionStateChevron> {
  Timer? _hide;
  late bool _visible = !widget.expanded;

  @override
  void didUpdateWidget(SectionStateChevron oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (widget.expanded == oldWidget.expanded) return;
    _visible = true;
    _hide?.cancel();
    if (!widget.expanded) return;
    _hide = Timer(const Duration(milliseconds: 900), () {
      setState(() => _visible = false);
    });
  }

  @override
  void dispose() {
    _hide?.cancel();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => ExcludeSemantics(
    child: Padding(
      padding: const EdgeInsetsDirectional.only(start: Grid.quarter),
      child: AnimatedOpacity(
        opacity: _visible ? 1 : 0,
        duration: MediaQuery.disableAnimationsOf(context)
            ? Duration.zero
            : Duration(milliseconds: _visible ? 120 : 180),
        child: Icon(
          widget.expanded ? BuzzIcons.chevronDown : BuzzIcons.chevronRight,
          size: 14,
          color: navigationAccessoryForeground(context),
        ),
      ),
    ),
  );
}
