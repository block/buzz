import 'package:flutter/material.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/mentions/agent_identity_provider.dart';
import '../../shared/profile/user_cache_provider.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/avatar_image.dart';
import 'agent_typing_bubble.dart';
import 'channel_identity_names_provider.dart';
import 'channel_typing_provider.dart';
import 'typing_avatar_cluster.dart';

/// One pending reply, shared by everyone currently typing.
class InlineAgentTyping extends ConsumerWidget {
  const InlineAgentTyping({
    super.key,
    required this.channelId,
    required this.entries,
    this.previewAvatars = const {},
  });
  final String channelId;
  final List<TypingEntry> entries;
  final Map<String, Widget> previewAvatars;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final keys = entries.map((entry) => entry.pubkey.toLowerCase()).toSet();
    if (keys.isEmpty) return const SizedBox.shrink();
    // Bound profile resolution and avatar construction independently of fanout.
    final visibleKeys = keys.take(keys.length > 4 ? 3 : 4).toList();
    final profiles = ref.watch(userCacheProvider);
    final bots = ref.watch(agentMentionPubkeysProvider(channelId));
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
      liveRegion: true,
      label: '$title ${keys.length == 1 ? 'is' : 'are'} typing',
      child: ExcludeSemantics(
        child: Padding(
          padding: const EdgeInsets.only(top: Grid.xs, bottom: Grid.xxs),
          child: Row(
            crossAxisAlignment: CrossAxisAlignment.end,
            children: [
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
              const SizedBox(width: messageAvatarContentGap),
              Expanded(
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
                    const AgentTypingBubble(),
                  ],
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}
