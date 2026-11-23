import 'package:flutter/material.dart';

import '../../../shared/theme/theme.dart';
import '../../../shared/theme/buzz_icons.dart';
import '../../../shared/widgets/app_list.dart';
import '../../../shared/widgets/app_list_card.dart';
import '../../../shared/widgets/avatar_image.dart';

/// The current Solo chat's agent and workspace, using standard settings rows.
class MeChatSettings extends StatelessWidget {
  const MeChatSettings({
    super.key,
    required this.agentName,
    required this.workspaceName,
    this.agentAvatarUrl,
    this.agentInitial = '?',
    this.hasAgent = false,
    this.renameRow,
    this.onAgentTap,
    this.onWorkspaceTap,
  });

  final Widget? renameRow;
  final String agentName;
  final String? agentAvatarUrl;
  final String agentInitial;
  final bool hasAgent;
  final String workspaceName;
  final VoidCallback? onAgentTap;
  final VoidCallback? onWorkspaceTap;

  @override
  Widget build(BuildContext context) {
    final agent = AppListRowRaw(
      leading: AvatarImage(
        imageUrl: agentAvatarUrl,
        radius: 16,
        isAgent: hasAgent,
        backgroundColor: context.controlSurface,
        fallback: hasAgent
            ? Text(agentInitial)
            : const Icon(BuzzIcons.plus, size: 20),
      ),
      title: Text('Agent', style: context.textTheme.bodyLarge),
      subtitle: Text(
        agentName,
        style: context.textTheme.bodySmall?.copyWith(
          color: context.colors.onSurfaceVariant,
        ),
      ),
      trailing: Icon(
        BuzzIcons.chevronRight,
        size: 18,
        color: context.colors.onSurfaceVariant,
      ),
      onTap: onAgentTap,
    );
    final workspace = AppListRowRaw(
      leading: SizedBox.square(
        dimension: 32,
        child: Icon(
          BuzzIcons.folder,
          size: 22,
          color: context.colors.onSurfaceVariant,
        ),
      ),
      title: Text(
        renameRow == null ? 'Workspace' : workspaceName,
        style: context.textTheme.bodyLarge,
      ),
      subtitle: renameRow != null
          ? null
          : Text(
              workspaceName,
              style: context.textTheme.bodySmall?.copyWith(
                color: context.colors.onSurfaceVariant,
              ),
            ),
      trailing: Icon(
        BuzzIcons.chevronRight,
        size: 18,
        color: context.colors.onSurfaceVariant,
      ),
      onTap: onWorkspaceTap,
    );
    AppListCard section(List<Widget> rows) => AppListCard(
      verticalPadding: Grid.twelve,
      dividerIndent: Grid.xs + Grid.md + Grid.xs,
      iconColumnWidth: Grid.md,
      children: rows,
    );
    final rename = renameRow;
    if (rename == null) return section([agent, workspace]);
    return Column(
      children: [
        section([agent]),
        section([rename, workspace]),
      ],
    );
  }
}
