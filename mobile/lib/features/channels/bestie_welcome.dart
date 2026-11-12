import 'package:flutter/material.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/profile/user_cache_provider.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/message_author_meta.dart';
import 'message_content.dart';
import 'small_avatar.dart';

/// A local onboarding bubble, never published or added to message history.
class BestieWelcome extends ConsumerWidget {
  const BestieWelcome({super.key, required this.agent});

  final String agent;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final profiles = ref.watch(userCacheProvider);
    final name = profiles[agent.toLowerCase()]?.label ?? 'your Bestie';
    // Agent names are literal text, not onboarding Markdown instructions.
    final escapedName = name.replaceAllMapped(
      RegExp(r'[\\`*_{}\[\]()<>#+.!|~-]'),
      (match) => '\\${match[0]}',
    );
    return Row(
      crossAxisAlignment: CrossAxisAlignment.end,
      children: [
        ExcludeSemantics(
          child: SmallAvatar(
            pubkey: agent,
            userCache: profiles,
            size: messageAvatarSize,
            agent: true,
          ),
        ),
        const SizedBox(width: messageAvatarContentGap),
        Expanded(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Padding(
                padding: const EdgeInsets.only(
                  left: Grid.xs,
                  bottom: Grid.quarter,
                ),
                child: MessageAuthorMeta(
                  nameStyle: messageUsernameTextStyle.copyWith(
                    fontSize: 13.1,
                    height: 17 / 13.1,
                  ),
                  displayName: name,
                  timestamp: '',
                  showTimestamp: false,
                  nameColor: context.colors.onSurface,
                  metadataColor: context.colors.onSurfaceVariant,
                ),
              ),
              MessageContent(
                bubble: true,
                baseStyle: messageBodyTextStyle.copyWith(
                  color: context.colors.onSurface,
                ),
                content:
                    'Hi, I’m $escapedName—welcome!\n\n'
                    'You’ve created your first agent. I’m saved as your Bestie '
                    'in the shortcut at the bottom of the app, and I’ll be the '
                    'default agent for new Solo chats in this space.\n\nYou can '
                    'change that anytime by tapping on the community avatar '
                    'in the top of the app and selecting **Agents**.\n\n'
                    'Whenever you’re ready, tell me what you’d like to work on.',
              ),
            ],
          ),
        ),
      ],
    );
  }
}
