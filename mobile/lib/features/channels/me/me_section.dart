import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

import '../../../shared/theme/theme.dart';
import '../../../shared/theme/buzz_icons.dart';
import '../../../shared/widgets/collapsible_section_body.dart';

/// A project or unassigned-chat section matching the Channels list headings.
class MeSection extends HookWidget {
  const MeSection({super.key, required this.label, required this.children});

  final String label;

  final List<Widget> children;

  @override
  Widget build(BuildContext context) {
    final expanded = useState(true);
    final color = navigationHeadingForeground(context);
    final reducedMotion = MediaQuery.disableAnimationsOf(context);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Semantics(
          label: label,
          excludeSemantics: true,
          onTap: () => expanded.value = !expanded.value,
          button: true,
          expanded: expanded.value,
          child: InkWell(
            borderRadius: BorderRadius.circular(Radii.md),
            onTap: () => expanded.value = !expanded.value,
            child: Padding(
              padding: const EdgeInsets.symmetric(
                horizontal: Grid.xxs,
                vertical: Grid.xxs,
              ),
              child: ConstrainedBox(
                constraints: const BoxConstraints(minHeight: 28),
                child: Row(
                  children: [
                    Flexible(
                      child: Text(
                        label,
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: navigationSectionTextStyle.copyWith(
                          color: color,
                        ),
                      ),
                    ),
                    const SizedBox(width: Grid.quarter),
                    AnimatedRotation(
                      turns: expanded.value ? 0 : -0.25,
                      duration: reducedMotion
                          ? Duration.zero
                          : sectionExpandDuration,
                      curve: sectionExpandCurve,
                      child: Icon(
                        BuzzIcons.chevronDown,
                        size: 14,
                        color: navigationAccessoryForeground(context),
                      ),
                    ),
                  ],
                ),
              ),
            ),
          ),
        ),
        CollapsibleSectionBody(
          expanded: expanded.value,
          child: Column(children: children),
        ),
      ],
    );
  }
}
