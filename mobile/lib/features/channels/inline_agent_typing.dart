import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/mentions/agent_identity_provider.dart';
import '../../shared/profile/user_cache_provider.dart';
import '../../shared/theme/theme.dart';
import '../../shared/theme/buzz_icons.dart';
import '../../shared/widgets/avatar_image.dart';
import 'agent_typing_bubble.dart';
import 'activity_disclosure_motion.dart';
import 'activity_disclosure_state.dart';
import 'agent_activity/inline_live_activity.dart';
import 'channel_management_provider.dart';
import 'message_author_visibility.dart';
import 'channel_identity_names_provider.dart';
import 'channel_typing_provider.dart';
import 'typing_avatar_cluster.dart';

/// One pending reply, shared by everyone currently typing.
class InlineAgentTyping extends HookConsumerWidget {
  const InlineAgentTyping({
    super.key,
    required this.channelId,
    required this.entries,
    this.previewAvatars = const {},
    this.knownAgents = const {},
  });
  final String channelId;
  final List<TypingEntry> entries;
  final Map<String, Widget> previewAvatars;

  /// Lowercase pubkeys known to be agents, such as the user's own.
  final Set<String> knownAgents;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final expanded = useState(false);
    final keys = entries.map((entry) => entry.pubkey.toLowerCase()).toSet();
    if (keys.isEmpty) return const SizedBox.shrink();
    // Bound profile resolution and avatar construction independently of fanout.
    final visibleKeys = keys.take(keys.length > 4 ? 3 : 4).toList();
    final scopes = {
      for (final entry in entries)
        (
          channel: channelId,
          author: entry.pubkey.toLowerCase(),
          root: entry.threadHeadId,
        ),
    };
    for (final scope in scopes) {
      ref.watch(pendingActivityExpansionProvider(scope));
    }
    void toggle() {
      expanded.value = !expanded.value;
      for (final scope in scopes) {
        ref
            .read(pendingActivityExpansionProvider(scope).notifier)
            .setExpanded(expanded.value);
      }
    }

    final profiles = ref.watch(userCacheProvider);
    final bots = ref.watch(agentMentionPubkeysProvider(channelId));
    final agents = keys
        .where(
          (key) =>
              knownAgents.contains(key) ||
              bots.contains(key) ||
              (profiles[key]?.isAgent ?? false),
        )
        .toList();
    final showIdentity =
        keys.length != 1 ||
        showMessageAuthorName(
          ref,
          channelId,
          keys.first,
          ref.watch(currentPubkeyProvider),
        );
    final labels = watchChannelIdentityLabels(
      ref,
      channelId,
      visibleKeys.where((key) => !previewAvatars.containsKey(key)),
    );
    final name = previewAvatars.containsKey(visibleKeys.first)
        ? 'Demo agent'
        : labels[visibleKeys.first] ?? 'Someone';
    final title = keys.length == 1
        ? name
        : '$name and ${keys.length - 1} ${keys.length == 2 ? 'other' : 'others'}';
    return Semantics(
      liveRegion: !expanded.value,
      explicitChildNodes: agents.isNotEmpty,
      excludeSemantics: agents.isEmpty,
      label: '$title ${keys.length == 1 ? 'is' : 'are'} typing',
      child: Padding(
        padding: const EdgeInsets.only(top: Grid.xs, bottom: Grid.xxs),
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.end,
          children: [
            if (showIdentity)
              TypingAvatarCluster(
                count: keys.length,
                avatars: {
                  for (final key in visibleKeys)
                    key:
                        previewAvatars[key] ??
                        AvatarImage(
                          key: ValueKey('inline-agent-typing-$key'),
                          imageUrl:
                              (profiles[key] ??
                                      ref
                                          .read(userCacheProvider.notifier)
                                          .get(key))
                                  ?.avatarUrl,
                          radius: messageAvatarSize / 2,
                          isAgent:
                              knownAgents.contains(key) ||
                              bots.contains(key) ||
                              (profiles[key]?.isAgent ?? false),
                          backgroundColor: context.controlSurface,
                          fallback: Text(
                            profiles[key]?.initial ??
                                key.substring(0, 1).toUpperCase(),
                          ),
                        ),
                },
              ),
            if (showIdentity) const SizedBox(width: messageAvatarContentGap),
            Expanded(
              child: ActivityMessageWidth(
                showAvatar: showIdentity,
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Padding(
                      padding: const EdgeInsets.only(left: Grid.xs),
                      child: Text(
                        title,
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: messageUsernameTextStyle.copyWith(
                          color: context.colors.onSurface,
                          fontSize: 13.1,
                          height: 17 / 13.1,
                        ),
                      ),
                    ),
                    const SizedBox(height: Grid.xxs),
                    if (agents.isEmpty)
                      const AgentTypingBubble()
                    else
                      ActivityDisclosureMotion(
                        expanded: expanded.value,
                        onToggle: toggle,
                        child: Material(
                          animationDuration: Duration.zero,
                          color: Colors.transparent,
                          child: Column(
                            mainAxisSize: MainAxisSize.min,
                            crossAxisAlignment: CrossAxisAlignment.start,
                            children: [
                              Semantics(
                                button: true,
                                expanded: expanded.value,
                                label:
                                    '${expanded.value ? 'Collapse' : 'Show'} live activity from $title',
                                child: InkWell(
                                  onTap: toggle,
                                  child: ConstrainedBox(
                                    constraints: const BoxConstraints(
                                      minHeight: 48,
                                      minWidth: 48,
                                    ),
                                    child: Padding(
                                      padding: expanded.value
                                          ? const EdgeInsets.all(Grid.xs)
                                          : EdgeInsets.zero,
                                      child: expanded.value
                                          ? const Row(
                                              mainAxisSize: MainAxisSize.min,
                                              children: [
                                                Icon(
                                                  BuzzIcons.chevronUp,
                                                  size: 18,
                                                ),
                                                SizedBox(width: Grid.xs),
                                                Text('Collapse'),
                                              ],
                                            )
                                          : const Align(
                                              alignment: Alignment.centerLeft,
                                              widthFactor: 1,
                                              heightFactor: 1,
                                              child: AgentTypingBubble(),
                                            ),
                                    ),
                                  ),
                                ),
                              ),
                              if (expanded.value)
                                for (final agent in agents)
                                  InlineLiveActivity(
                                    channelId: channelId,
                                    agentPubkey: agent,
                                    threadRoots: {
                                      for (final entry in entries)
                                        if (entry.pubkey.toLowerCase() == agent)
                                          entry.threadHeadId,
                                    },
                                  ),
                            ],
                          ),
                        ),
                      ),
                  ],
                ),
              ),
            ),
          ],
        ),
      ),
    );
  }
}
