import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

import '../../../shared/theme/theme.dart';
import '../../../shared/widgets/collapsible_section_body.dart';
import '../../../shared/widgets/collapsible_sliver_body.dart';

import '../channel_sections/section_options_button.dart';
import '../channels_page/filters.dart';

/// A project or unassigned-chat section matching the Channels list headings.
class MeSection extends HookWidget {
  const MeSection({
    super.key,
    required this.label,
    required this.children,
    this.sliver = false,
    this.compactHeaderGap = false,
    this.onFilter,
  }) : buildBody = null;

  const MeSection.sliverBody({
    super.key,
    required this.label,
    required this.buildBody,
    this.onFilter,
    this.compactHeaderGap = false,
  }) : sliver = true,
       children = const [];

  final String label;
  final VoidCallback? onFilter;
  final bool sliver;
  final bool compactHeaderGap;
  final List<Widget> children;
  final Widget Function(bool expanded)? buildBody;

  @override
  Widget build(BuildContext context) {
    final expanded = useState(true);
    final inset = compactHeaderGap
        ? 26.0 - MessagesSectionOptions.headerTrim(context)
        : Grid.xxs;
    final toggle = Semantics(
      label: label,
      excludeSemantics: true,
      onTap: () => expanded.value = !expanded.value,
      button: true,
      expanded: expanded.value,
      child: InkWell(
        borderRadius: BorderRadius.circular(Radii.md),
        onTap: () => expanded.value = !expanded.value,
        child: Padding(
          padding: EdgeInsets.fromLTRB(
            Grid.xxs,
            inset,
            Grid.xxs,
            compactHeaderGap && expanded.value ? 0 : inset,
          ),
          child: ConstrainedBox(
            constraints: BoxConstraints(minHeight: compactHeaderGap ? 0 : 28),
            child: Row(
              crossAxisAlignment: compactHeaderGap
                  ? CrossAxisAlignment.end
                  : CrossAxisAlignment.center,
              children: [
                Flexible(
                  child: Text(
                    label,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: navigationBucketHeadingStyle(context),
                  ),
                ),
              ],
            ),
          ),
        ),
      ),
    );
    final header = onFilter == null
        ? toggle
        : Row(
            children: [
              Expanded(child: toggle),
              Padding(
                padding: EdgeInsets.zero,
                child: SectionOptionsButton(
                  tooltip: '$label options',
                  onPressed: onFilter!,
                ),
              ),
            ],
          );
    if (sliver) {
      return SliverMainAxisGroup(
        slivers: [
          SliverToBoxAdapter(
            child: Padding(
              padding: EdgeInsets.only(
                left: MessagesSectionOptions.leadingInset(context) - Grid.xxs,
                right: Grid.gutter - Grid.xxs,
              ),
              child: header,
            ),
          ),
          CollapsibleSliverBody(
            expanded: expanded.value,
            sliver: SliverList.list(children: children),
          ),
        ],
      );
    }
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        header,
        CollapsibleSectionBody(
          expanded: expanded.value,
          child: Column(children: children),
        ),
      ],
    );
  }
}
