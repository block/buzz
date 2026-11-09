import 'package:flutter/material.dart';

import '../../shared/theme/theme.dart';
import '../../shared/widgets/app_list.dart';
import '../../shared/widgets/builderlab_logo_icon.dart';
import '../../shared/widgets/modal_presentation.dart';

/// Shows where a new agent runs; mobile only creates Builderlab agents.
class BuilderlabHarnessRow extends StatelessWidget {
  const BuilderlabHarnessRow({super.key});

  @override
  Widget build(BuildContext context) {
    void explain() => showBuzzModalBottomSheet<void>(
      context: context,
      title: 'Builderlab',
      showDragHandle: true,
      builder: (_) => const _BuilderlabSheet(),
    );
    return Semantics(
      button: true,
      label: 'Harness: Builderlab',
      onTap: explain,
      excludeSemantics: true,
      child: AppListRow(
        title: 'Harness',
        value: 'Builderlab',
        onTap: explain,
        trailing: Icon(
          Icons.unfold_more,
          size: 16,
          color: context.colors.onSurfaceVariant,
        ),
      ),
    );
  }
}

class _BuilderlabSheet extends StatelessWidget {
  const _BuilderlabSheet();

  @override
  Widget build(BuildContext context) => SafeArea(
    top: false,
    child: Padding(
      padding: const EdgeInsets.fromLTRB(
        Grid.gutter,
        Grid.xxs,
        Grid.gutter,
        Grid.xs,
      ),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          BuilderlabLogoIcon(size: 40, color: context.colors.onSurface),
          const SizedBox(height: Grid.xs),
          Text(
            'Agents you create on your phone run remotely on Builderlab. To '
            'use another harness, create the agent in Buzz on your computer.',
            textAlign: TextAlign.center,
            style: context.textTheme.bodyMedium?.copyWith(
              color: context.colors.onSurfaceVariant,
            ),
          ),
        ],
      ),
    ),
  );
}
