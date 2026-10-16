import 'package:flutter/material.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/profile/user_cache_provider.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/avatar_image.dart';
import 'channel_identity_names_provider.dart';
import 'channel_typing_provider.dart';
import 'agent_typing_bubble.dart';

/// Ephemeral personal-chat message rows, using desktop's three-dot work signal.
class InlineAgentTyping extends ConsumerWidget {
  const InlineAgentTyping({
    super.key,
    required this.channelId,
    required this.entries,
  });
  final String channelId;
  final List<TypingEntry> entries;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final keys = entries.map((entry) => entry.pubkey.toLowerCase()).toSet();
    final profiles = ref.watch(userCacheProvider);
    final labels = watchChannelIdentityLabels(ref, channelId, keys);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        for (final key in keys)
          Semantics(
            key: ValueKey('inline-agent-typing-$key'),
            liveRegion: true,
            label: '${labels[key]} is working',
            child: ExcludeSemantics(
              child: Padding(
                padding: const EdgeInsets.only(top: Grid.xs, bottom: Grid.xxs),
                child: Row(
                  crossAxisAlignment: CrossAxisAlignment.end,
                  children: [
                    AvatarImage(
                      imageUrl:
                          (profiles[key] ??
                                  ref.read(userCacheProvider.notifier).get(key))
                              ?.avatarUrl,
                      radius: messageAvatarSize / 2,
                      isAgent: true,
                      backgroundColor: context.controlSurface,
                      fallback: Text(
                        profiles[key]?.initial ??
                            key.substring(0, 1).toUpperCase(),
                      ),
                    ),
                    const SizedBox(width: messageAvatarContentGap),
                    Expanded(
                      child: Column(
                        crossAxisAlignment: CrossAxisAlignment.start,
                        children: [
                          Padding(
                            padding: const EdgeInsets.only(left: Grid.xs),
                            child: Text(
                              labels[key] ?? 'Assistant',
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
          ),
      ],
    );
  }
}
