import 'package:flutter/material.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../../shared/mentions/agent_identity_provider.dart';
import '../../../shared/profile/user_cache_provider.dart';
import '../../../shared/theme/theme.dart';
import '../../../shared/theme/buzz_icons.dart';
import '../../../shared/widgets/app_list.dart';
import '../../../shared/widgets/avatar_image.dart';
import '../../../shared/widgets/sheet_action_section.dart';

/// The grouped member-style assistant chooser shared by personal chat sheets.
class MeAssistantList extends ConsumerWidget {
  const MeAssistantList({
    super.key,
    required this.choices,
    required this.selected,
  });

  final List<AgentDirectoryEntry> choices;
  final String? selected;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final profiles = ref.watch(userCacheProvider);
    return SizedBox(
      height: (MediaQuery.sizeOf(context).height * 0.6).clamp(0.0, 400.0),
      child: choices.isEmpty
          ? const Center(child: Text('No agents are available.'))
          : ListView(
              padding: const EdgeInsets.fromLTRB(
                Grid.gutter,
                0,
                Grid.gutter,
                Grid.xs,
              ),
              children: [
                SheetActionSection(
                  label: 'Agents · ${choices.length}',
                  dividerIndent: Grid.xs + 40 + Grid.xs,
                  children: [
                    for (final agent in choices)
                      Semantics(
                        selected: agent.pubkey == selected,
                        child: AppListRowRaw(
                          leading: AvatarImage(
                            imageUrl:
                                (profiles[agent.pubkey] ??
                                        ref
                                            .read(userCacheProvider.notifier)
                                            .get(agent.pubkey))
                                    ?.avatarUrl,
                            radius: 20,
                            isAgent: true,
                            backgroundColor: context.controlSurface,
                            fallback: Text(
                              agent.displayName?.isNotEmpty == true
                                  ? agent.displayName![0]
                                  : 'A',
                            ),
                          ),
                          title: Text(agent.displayName ?? 'Agent'),
                          trailing: agent.pubkey == selected
                              ? const Icon(BuzzIcons.check)
                              : null,
                          onTap: () => Navigator.pop(context, agent.pubkey),
                        ),
                      ),
                  ],
                ),
              ],
            ),
    );
  }
}
