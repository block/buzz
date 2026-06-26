import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/theme/theme.dart';
import '../custom_emoji/custom_emoji_render.dart';
import '../profile/user_cache_provider.dart';
import '../profile/user_profile.dart';
import 'channel_management_provider.dart';
import 'timeline_message.dart';

/// Toggle a reaction on [message]. If the current user already reacted with
/// [emoji], removes the reaction; otherwise adds it.
///
/// Used by channel detail, thread detail, and system message rows to avoid
/// duplicating the toggle wiring.
void toggleReaction(WidgetRef ref, TimelineMessage message, String emoji) {
  final actions = ref.read(channelActionsProvider);
  final reaction = message.reactions.firstWhere((r) => r.emoji == emoji);
  if (reaction.reactedByCurrentUser && reaction.currentUserReactionId != null) {
    actions.removeReaction(reaction.currentUserReactionId!, emoji);
  } else {
    actions.addReaction(message.id, emoji);
  }
}

class ReactionRow extends StatelessWidget {
  final List<TimelineReaction> reactions;
  final void Function(String emoji) onToggle;

  const ReactionRow({
    super.key,
    required this.reactions,
    required this.onToggle,
  });

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.only(top: Grid.half),
      child: Wrap(
        spacing: Grid.half,
        runSpacing: Grid.half,
        children: [
          for (final reaction in reactions)
            GestureDetector(
              onTap: () => onToggle(reaction.emoji),
              onLongPress: () => showReactionDetailSheet(
                context: context,
                reactions: reactions,
                initialEmoji: reaction.emoji,
              ),
              child: Container(
                padding: const EdgeInsets.symmetric(
                  horizontal: Grid.xxs,
                  vertical: Grid.quarter,
                ),
                decoration: BoxDecoration(
                  color: reaction.reactedByCurrentUser
                      ? context.colors.primary.withValues(alpha: 0.12)
                      : context.colors.surfaceContainerHighest,
                  borderRadius: BorderRadius.circular(Radii.lg),
                  border: Border.all(
                    color: reaction.reactedByCurrentUser
                        ? context.colors.primary.withValues(alpha: 0.4)
                        : context.colors.outlineVariant,
                  ),
                ),
                child: Row(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    _ReactionEmoji(reaction: reaction, size: 18),
                    if (reaction.count > 1) ...[
                      const SizedBox(width: Grid.quarter),
                      Text(
                        '${reaction.count}',
                        style: context.textTheme.labelSmall?.copyWith(
                          color: reaction.reactedByCurrentUser
                              ? context.colors.primary
                              : context.colors.onSurfaceVariant,
                          fontWeight: FontWeight.w600,
                        ),
                      ),
                    ],
                  ],
                ),
              ),
            ),
        ],
      ),
    );
  }
}

class _ReactionEmoji extends StatelessWidget {
  final TimelineReaction reaction;
  final double size;

  const _ReactionEmoji({required this.reaction, required this.size});

  @override
  Widget build(BuildContext context) {
    final emojiUrl = reaction.emojiUrl;
    if (emojiUrl == null || emojiUrl.isEmpty) {
      return Text(reaction.emoji, style: TextStyle(fontSize: size));
    }
    final shortcode = reaction.emoji.substring(1, reaction.emoji.length - 1);
    return CustomEmojiImage(shortcode: shortcode, url: emojiUrl, size: size);
  }
}

void showReactionDetailSheet({
  required BuildContext context,
  required List<TimelineReaction> reactions,
  required String initialEmoji,
}) {
  showModalBottomSheet<void>(
    context: context,
    isScrollControlled: true,
    showDragHandle: true,
    backgroundColor: Theme.of(context).colorScheme.surfaceContainerHighest,
    builder: (sheetContext) =>
        _ReactionDetailSheet(reactions: reactions, initialEmoji: initialEmoji),
  );
}

class _ReactionDetailSheet extends HookConsumerWidget {
  final List<TimelineReaction> reactions;
  final String initialEmoji;

  const _ReactionDetailSheet({
    required this.reactions,
    required this.initialEmoji,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final selectedEmoji = useState(initialEmoji);
    final userCache = ref.watch(userCacheProvider);

    final currentReaction = reactions.firstWhere(
      (r) => r.emoji == selectedEmoji.value,
      orElse: () => reactions.first,
    );

    // Preload profiles for reactors.
    useEffect(() {
      if (currentReaction.userPubkeys.isNotEmpty) {
        ref
            .read(userCacheProvider.notifier)
            .preload(currentReaction.userPubkeys);
      }
      return null;
    }, [currentReaction.userPubkeys]);

    return ConstrainedBox(
      constraints: BoxConstraints(
        maxHeight: MediaQuery.sizeOf(context).height * 0.5,
      ),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          // Emoji filter chips (if multiple reaction types).
          if (reactions.length > 1)
            Padding(
              padding: const EdgeInsets.symmetric(horizontal: Grid.xs),
              child: SingleChildScrollView(
                scrollDirection: Axis.horizontal,
                child: Row(
                  children: [
                    for (final reaction in reactions)
                      Padding(
                        padding: const EdgeInsets.only(right: Grid.half),
                        child: ChoiceChip(
                          label: Row(
                            mainAxisSize: MainAxisSize.min,
                            children: [
                              _ReactionEmoji(reaction: reaction, size: 16),
                              const SizedBox(width: Grid.quarter),
                              Text('${reaction.count}'),
                            ],
                          ),
                          selected: reaction.emoji == selectedEmoji.value,
                          onSelected: (_) {
                            selectedEmoji.value = reaction.emoji;
                          },
                        ),
                      ),
                  ],
                ),
              ),
            ),

          // Header: emoji + shortcode.
          Padding(
            padding: const EdgeInsets.symmetric(
              horizontal: Grid.xs,
              vertical: Grid.half,
            ),
            child: Row(
              children: [
                _ReactionEmoji(reaction: currentReaction, size: 32),
                const SizedBox(width: Grid.half),
                Text(
                  currentReaction.emoji.startsWith(':')
                      ? currentReaction.emoji
                      : ':${_emojiToShortcode(currentReaction.emoji)}:',
                  style: context.textTheme.titleSmall?.copyWith(
                    color: context.colors.onSurfaceVariant,
                  ),
                ),
              ],
            ),
          ),

          const Divider(height: 1),

          // Reactor list.
          Flexible(
            child: ListView.builder(
              shrinkWrap: true,
              padding: EdgeInsets.only(
                top: Grid.half,
                bottom: MediaQuery.viewPaddingOf(context).bottom + Grid.half,
              ),
              itemCount: currentReaction.userPubkeys.length,
              itemBuilder: (context, index) {
                final pubkey = currentReaction.userPubkeys[index];
                final profile = userCache[pubkey.toLowerCase()];
                return _ReactorTile(profile: profile, pubkey: pubkey);
              },
            ),
          ),
        ],
      ),
    );
  }
}

class _ReactorTile extends StatelessWidget {
  final UserProfile? profile;
  final String pubkey;

  const _ReactorTile({required this.profile, required this.pubkey});

  @override
  Widget build(BuildContext context) {
    final displayName =
        profile?.label ??
        (pubkey.length >= 8 ? '${pubkey.substring(0, 8)}...' : pubkey);
    final about = profile?.about;

    return ListTile(
      leading: _ReactorAvatar(
        avatarUrl: profile?.avatarUrl,
        initial:
            profile?.initial ??
            (pubkey.isNotEmpty ? pubkey[0].toUpperCase() : '?'),
      ),
      title: Text(
        displayName,
        style: context.textTheme.bodyMedium?.copyWith(
          fontWeight: FontWeight.w600,
        ),
        overflow: TextOverflow.ellipsis,
      ),
      subtitle: about != null && about.isNotEmpty
          ? Text(
              about,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: context.textTheme.bodySmall?.copyWith(
                color: context.colors.onSurfaceVariant,
              ),
            )
          : null,
      dense: true,
    );
  }
}

class _ReactorAvatar extends HookWidget {
  final String? avatarUrl;
  final String initial;

  const _ReactorAvatar({required this.avatarUrl, required this.initial});

  @override
  Widget build(BuildContext context) {
    final failed = useState(false);

    useEffect(() {
      failed.value = false;
      return null;
    }, [avatarUrl]);

    final url = avatarUrl;
    if (url == null || failed.value) {
      return CircleAvatar(child: Text(initial));
    }
    return CircleAvatar(
      backgroundImage: NetworkImage(url),
      onBackgroundImageError: (_, _) => failed.value = true,
      child: null,
    );
  }
}

String _emojiToShortcode(String emoji) {
  const map = <String, String>{
    '👍': 'thumbsup',
    '👎': 'thumbsdown',
    '❤️': 'heart',
    '🎉': 'tada',
    '😂': 'joy',
    '😢': 'cry',
    '😮': 'open_mouth',
    '🔥': 'fire',
    '👀': 'eyes',
    '🚀': 'rocket',
    '💯': '100',
    '👏': 'clap',
    '🙏': 'pray',
    '💬': 'speech_balloon',
    '✅': 'white_check_mark',
    '❌': 'x',
    '⭐': 'star',
    '🤔': 'thinking',
    '😄': 'smile',
    '😎': 'sunglasses',
    '🤣': 'rofl',
    '😍': 'heart_eyes',
    '🥳': 'partying_face',
    '👋': 'wave',
    '💪': 'muscle',
    '🙌': 'raised_hands',
    '😅': 'sweat_smile',
    '🫡': 'saluting_face',
  };
  return map[emoji] ?? 'emoji';
}
