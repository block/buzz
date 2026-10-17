import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

import '../../../shared/theme/theme.dart';
import '../../../shared/theme/buzz_icons.dart';
import '../../../shared/widgets/collapsible_section_body.dart';

/// A project or unassigned-chat section matching the Channels list headings.
class MeSection extends HookWidget {
  const MeSection({
    super.key,
    required this.label,
    required this.icon,
    required this.children,
    this.hasUnread = false,
  });

  final String label;

  /// Keeps unread activity visible when this section is collapsed.
  final bool hasUnread;
  final IconData icon;
  final List<Widget> children;

  @override
  Widget build(BuildContext context) {
    final expanded = useState(true);
    final color = navigationSectionForeground(context);
    final reducedMotion = MediaQuery.disableAnimationsOf(context);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Semantics(
          label: hasUnread ? '$label, unread' : label,
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
                vertical: Grid.twelve,
              ),
              child: ConstrainedBox(
                constraints: const BoxConstraints(minHeight: 28),
                child: Row(
                  children: [
                    SizedBox(
                      width: 22,
                      child: Align(
                        alignment: Alignment.centerLeft,
                        child: Icon(icon, size: 18, color: color),
                      ),
                    ),
                    const SizedBox(width: Grid.xxs),
                    Expanded(
                      child: Text(
                        label,
                        style: contentListTitleTextStyle.copyWith(
                          color: color,
                          fontSize: 17,
                          height: 22 / 17,
                          fontWeight: FontWeight.w600,
                        ),
                      ),
                    ),
                    if (hasUnread) ...[
                      Container(
                        key: ValueKey('solo-section-unread-$label'),
                        width: 8,
                        height: 8,
                        decoration: BoxDecoration(
                          color: context.colors.primary,
                          shape: BoxShape.circle,
                        ),
                      ),
                      const SizedBox(width: Grid.xxs),
                    ],
                    AnimatedRotation(
                      turns: expanded.value ? 0 : -0.25,
                      duration: reducedMotion
                          ? Duration.zero
                          : sectionExpandDuration,
                      curve: sectionExpandCurve,
                      child: Icon(
                        BuzzIcons.chevronDown,
                        size: 18,
                        color: color,
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
