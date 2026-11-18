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
    this.onAgentTap,
    this.onWorkspaceTap,
  });

  final String agentName;
  final String? agentAvatarUrl;
  final String agentInitial;
  final bool hasAgent;
  final String workspaceName;
  final VoidCallback? onAgentTap;
  final VoidCallback? onWorkspaceTap;

  @override
  Widget build(BuildContext context) => AppListCard(
    verticalPadding: Grid.twelve,
    dividerIndent: Grid.xs + 32 + Grid.xs,
    children: [
      AppListRowRaw(
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
      ),
      AppListRowRaw(
        leading: SizedBox.square(
          dimension: 32,
          child: Icon(
            BuzzIcons.workspace,
            size: 22,
            color: context.colors.onSurfaceVariant,
          ),
        ),
        title: Text('Workspace', style: context.textTheme.bodyLarge),
        subtitle: Text(
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
      ),
    ],
  );
}
