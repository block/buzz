import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../../shared/theme/theme.dart';
import '../../shared/widgets/avatar_image.dart';
import '../../shared/widgets/buzz_loading_indicator.dart';
import '../../shared/widgets/frosted_app_bar.dart';
import '../../shared/widgets/frosted_scaffold.dart';
import '../channels/mentions/mention_candidates.dart';
import '../channels/mentions/mention_candidates_provider.dart';
import '../profile/presence_cache_provider.dart';
import '../profile/profile_provider.dart';
import '../profile/user_cache_provider.dart';
import '../profile/user_profile.dart';
import '../profile/user_profile_sheet.dart';

/// Read-only mobile directory for agents advertised by the current community.
class AgentsPage extends HookConsumerWidget {
  const AgentsPage({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final agentsAsync = ref.watch(agentDirectoryProvider);
    final owners = ref.watch(agentOwnersProvider).asData?.value ?? const {};
    final profiles = ref.watch(userCacheProvider);
    final presence = ref.watch(presenceCacheProvider);
    final currentPubkey = ref.watch(profileProvider).asData?.value?.pubkey;
    final agents = agentsAsync.asData?.value ?? const <AgentDirectoryEntry>[];
    final pubkeys = agents.map((agent) => agent.pubkey).toSet().toList()
      ..sort();
    final pubkeysKey = pubkeys.join(',');

    useEffect(() {
      if (pubkeys.isEmpty) return null;
      ref.read(userCacheProvider.notifier).preload(pubkeys);
      ref.read(presenceCacheProvider.notifier).track(pubkeys);
      return null;
    }, [pubkeysKey]);

    return FrostedScaffold(
      key: const Key('agents-page'),
      appBar: const FrostedAppBar(
        automaticallyImplyLeading: false,
        title: Text('Agents'),
      ),
      body: Padding(
        padding: EdgeInsets.only(top: frostedAppBarHeight(context)),
        child: agentsAsync.when(
          loading: () => const Center(
            child: BuzzLoadingIndicator(
              size: 40,
              semanticLabel: 'Loading agents',
            ),
          ),
          error: (_, _) => _AgentsStatus(
            icon: LucideIcons.circleAlert,
            message: 'Could not load agents.',
            actionLabel: 'Retry',
            onAction: () => ref.invalidate(agentDirectoryProvider),
          ),
          data: (rawAgents) {
            final agentsByPubkey = {
              for (final agent in rawAgents) agent.pubkey: agent,
            };
            final visibleAgents = agentsByPubkey.values.toList()
              ..sort((a, b) {
                final aName =
                    profiles[a.pubkey]?.label ?? a.displayName ?? a.pubkey;
                final bName =
                    profiles[b.pubkey]?.label ?? b.displayName ?? b.pubkey;
                return aName.toLowerCase().compareTo(bName.toLowerCase());
              });

            if (visibleAgents.isEmpty) {
              return const _AgentsStatus(
                icon: LucideIcons.bot,
                message: 'No agents are advertising in this community yet.',
              );
            }

            return ListView.separated(
              padding: const EdgeInsets.fromLTRB(
                Grid.gutter,
                Grid.xxs,
                Grid.gutter,
                Grid.gutter,
              ),
              itemCount: visibleAgents.length,
              separatorBuilder: (_, _) => const Divider(height: 1),
              itemBuilder: (context, index) {
                final agent = visibleAgents[index];
                final profile = profiles[agent.pubkey];
                final owner = owners[agent.pubkey] ?? profile?.ownerPubkey;
                final ownerLabel = owner == null
                    ? 'Community agent'
                    : currentPubkey != null &&
                          owner.toLowerCase() == currentPubkey.toLowerCase()
                    ? 'Managed by you'
                    : 'Externally managed';

                return _AgentRow(
                  agent: agent,
                  profile: profile,
                  ownerLabel: ownerLabel,
                  presence: presence[agent.pubkey] ?? 'offline',
                  onTap: () => showUserProfileSheet(context, agent.pubkey),
                );
              },
            );
          },
        ),
      ),
    );
  }
}

class _AgentRow extends StatelessWidget {
  const _AgentRow({
    required this.agent,
    required this.profile,
    required this.ownerLabel,
    required this.presence,
    required this.onTap,
  });

  final AgentDirectoryEntry agent;
  final UserProfile? profile;
  final String ownerLabel;
  final String presence;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final name =
        profile?.label ?? agent.displayName ?? _shortPubkey(agent.pubkey);
    final handle = profile?.nip05Handle?.trim();

    return ListTile(
      contentPadding: const EdgeInsets.symmetric(vertical: Grid.half),
      leading: Stack(
        clipBehavior: Clip.none,
        children: [
          AvatarImage(
            imageUrl: profile?.avatarUrl,
            radius: Grid.gutter,
            backgroundColor: context.colors.primaryContainer,
            fallback: Text(
              profile?.initial ?? name.characters.first.toUpperCase(),
              style: context.textTheme.labelLarge?.copyWith(
                color: context.colors.onPrimaryContainer,
              ),
            ),
          ),
          Positioned(
            right: -1,
            bottom: -1,
            child: Container(
              width: 10,
              height: 10,
              decoration: BoxDecoration(
                color: _presenceColor(context, presence),
                shape: BoxShape.circle,
                border: Border.all(color: context.colors.surface, width: 2),
              ),
            ),
          ),
        ],
      ),
      title: Text(name, maxLines: 1, overflow: TextOverflow.ellipsis),
      subtitle: Text(
        handle == null || handle.isEmpty ? ownerLabel : '$handle · $ownerLabel',
        maxLines: 1,
        overflow: TextOverflow.ellipsis,
      ),
      trailing: const Icon(LucideIcons.chevronRight, size: 18),
      onTap: onTap,
    );
  }
}

class _AgentsStatus extends StatelessWidget {
  const _AgentsStatus({
    required this.icon,
    required this.message,
    this.actionLabel,
    this.onAction,
  });

  final IconData icon;
  final String message;
  final String? actionLabel;
  final VoidCallback? onAction;

  @override
  Widget build(BuildContext context) {
    return Center(
      child: Padding(
        padding: const EdgeInsets.all(Grid.gutter),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Icon(icon, size: Grid.xl, color: context.colors.onSurfaceVariant),
            const SizedBox(height: Grid.xs),
            Text(
              message,
              textAlign: TextAlign.center,
              style: context.textTheme.bodyMedium?.copyWith(
                color: context.colors.onSurfaceVariant,
              ),
            ),
            if (actionLabel != null && onAction != null) ...[
              const SizedBox(height: Grid.xs),
              TextButton(onPressed: onAction, child: Text(actionLabel!)),
            ],
          ],
        ),
      ),
    );
  }
}

String _shortPubkey(String pubkey) =>
    pubkey.length <= 8 ? pubkey : '${pubkey.substring(0, 8)}…';

Color _presenceColor(BuildContext context, String presence) =>
    switch (presence) {
      'online' => context.appColors.success,
      'away' => context.appColors.warning,
      _ => context.colors.outline,
    };
