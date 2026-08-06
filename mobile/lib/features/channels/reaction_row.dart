import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../../shared/theme/theme.dart';
import '../../shared/widgets/avatar_image.dart';
import '../../shared/widgets/modal_presentation.dart';
import '../../shared/custom_emoji/custom_emoji_render.dart';
import '../../shared/emoji/emoji_burst.dart';
import '../../shared/emoji/emoji_data_provider.dart';
import '../../shared/emoji/native_emoji_glyph.dart';
import '../../shared/emoji/positive_emoji.dart';
import '../profile/user_cache_provider.dart';
import '../profile/user_profile.dart';
import 'channel_management_provider.dart';
import 'emoji_picker.dart';
import 'recent_emoji_provider.dart';
import 'timeline_message.dart';

/// Pill geometry, ported from desktop's `REACTION_PILL_BASE_CLASSES` in
/// `desktop/src/features/messages/ui/MessageReactions.tsx`: a fully-rounded
/// 28px pill with a fixed minimum width so a row of single-count reactions
/// doesn't look ragged.
const _pillHeight = 28.0;
const _pillMinWidth = 48.0;
const _pillGap = 6.0;

/// The `+` pill is narrower than a reaction pill — desktop's `w-10` vs `min-w-12`.
const _addPillWidth = 40.0;

/// Glyph size inside a pill. Desktop uses 12px in a 28px pill; mobile steps that
/// up because the app's base type is 15sp and a 12px glyph reads as a smudge at
/// phone viewing distance.
const _pillGlyphSize = 16.0;

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
    // Only adding counts as a use — removing a reaction shouldn't promote the
    // emoji in the frequently-used ranking.
    ref.read(recentEmojiProvider.notifier).record(emoji);
    actions.addReaction(message.id, emoji);
  }
}

/// Arm a burst for a reaction that is about to be added from somewhere the pill
/// doesn't exist yet — the quick-reaction row or the emoji picker.
///
/// Pill taps don't go through here: they burst immediately, because the pill is
/// already on screen. Adding a reaction you already have is a no-op on the
/// relay, so it shouldn't celebrate either.
void armReactionBurst(WidgetRef ref, TimelineMessage message, String emoji) {
  final alreadyReacted = message.reactions.any(
    (reaction) => reaction.emoji == emoji && reaction.reactedByCurrentUser,
  );
  if (alreadyReacted) return;
  ref.read(pendingReactionBurstProvider.notifier).arm(message.id, emoji);
}

/// Open the emoji picker and add whatever the user chooses as a reaction to
/// [message].
///
/// Shared by the `+` pill in the channel timeline and in the thread view so the
/// recency bookkeeping and burst arming can't drift apart between them.
void showAddReactionPicker({
  required BuildContext context,
  required WidgetRef ref,
  required TimelineMessage message,
}) {
  showEmojiPicker(
    context: context,
    onSelect: (emoji) {
      ref.read(recentEmojiProvider.notifier).record(emoji);
      armReactionBurst(ref, message, emoji);
      ref.read(channelActionsProvider).addReaction(message.id, emoji);
    },
  );
}

class ReactionRow extends StatelessWidget {
  /// The message these reactions belong to. Used to match a pending burst to
  /// the right pill — the same emoji can be pending on a different message.
  final String messageId;

  final List<TimelineReaction> reactions;
  final void Function(String emoji) onToggle;

  /// Show a trailing `+` pill that opens the emoji picker, mirroring desktop's
  /// `InlineReactionPicker`. Thread messages set this so reacting is always one
  /// tap away; the channel timeline leaves it off to keep the list dense.
  final bool showAddButton;

  /// Invoked by the `+` pill. Required when [showAddButton] is set.
  final VoidCallback? onAddReaction;

  const ReactionRow({
    super.key,
    required this.messageId,
    required this.reactions,
    required this.onToggle,
    this.showAddButton = false,
    this.onAddReaction,
  });

  @override
  Widget build(BuildContext context) {
    // With the add button on, an empty row still renders — that affordance is
    // the point. Without it, an empty row would be dead space.
    if (reactions.isEmpty && !showAddButton) return const SizedBox.shrink();

    return Padding(
      padding: const EdgeInsets.only(top: Grid.half),
      child: Wrap(
        spacing: Grid.half,
        runSpacing: Grid.half,
        children: [
          for (final reaction in reactions)
            _ReactionPill(
              messageId: messageId,
              reaction: reaction,
              onTap: () => onToggle(reaction.emoji),
              onLongPress: () => showReactionDetailSheet(
                context: context,
                reactions: reactions,
                initialEmoji: reaction.emoji,
              ),
            ),
          if (showAddButton && onAddReaction != null)
            _AddReactionPill(onTap: onAddReaction!),
        ],
      ),
    );
  }
}

/// Shared pill chrome so the reaction pills and the `+` pill can't drift.
class _PillSurface extends StatelessWidget {
  final bool highlighted;
  final double minWidth;
  final Widget child;

  const _PillSurface({
    required this.highlighted,
    required this.minWidth,
    required this.child,
  });

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    return Container(
      height: _pillHeight,
      // No `alignment:` — it wraps the child in an expanding Align, which makes
      // every pill fill the Wrap's full width and stack one per line.
      // Children center themselves instead.
      constraints: BoxConstraints(minWidth: minWidth),
      padding: const EdgeInsets.symmetric(horizontal: Grid.xxs),
      decoration: BoxDecoration(
        color: highlighted
            ? colors.primary.withValues(alpha: 0.10)
            : colors.surfaceContainerHighest,
        borderRadius: BorderRadius.circular(_pillHeight / 2),
        border: Border.all(
          color: highlighted
              ? colors.primary.withValues(alpha: 0.40)
              : colors.outlineVariant,
        ),
      ),
      child: child,
    );
  }
}

class _ReactionPill extends HookConsumerWidget {
  final String messageId;
  final TimelineReaction reaction;
  final VoidCallback onTap;
  final VoidCallback onLongPress;

  const _ReactionPill({
    required this.messageId,
    required this.reaction,
    required this.onTap,
    required this.onLongPress,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final reacted = reaction.reactedByCurrentUser;

    // Fire a burst armed elsewhere (quick-reaction row, emoji picker) now that
    // the pill it belongs to is finally on screen. Keyed on [reacted] rather
    // than watching the provider: watching would rebuild every pill in the
    // timeline each time a burst is armed or claimed, and the only moments that
    // matter are this pill's first build and the frame our own reaction lands.
    useEffect(() {
      if (!reacted) return null;
      final target = PendingReactionBurst(
        messageId: messageId,
        emoji: reaction.emoji,
      );
      if (ref.read(pendingReactionBurstProvider) != target) return null;
      // Post-frame so the pill has been laid out and can report its centre.
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (!context.mounted) return;
        // The same message is mounted on two routes at once when a thread is
        // pushed over its channel, and the channel's copy builds first. Without
        // this the channel pill claims the burst and it only becomes visible
        // after popping back — the burst has to play where the user is looking.
        final route = ModalRoute.of(context);
        if (route != null && !route.isCurrent) return;
        if (!ref.read(pendingReactionBurstProvider.notifier).claim(target)) {
          return;
        }
        burstEmojiFromContext(ref, context, reaction.emoji);
      });
      return null;
    }, [reacted, messageId, reaction.emoji]);

    return GestureDetector(
      key: ValueKey('reaction-pill-${reaction.emoji}'),
      onTap: () {
        // The pill is already here, so burst straight away rather than waiting
        // for the relay echo — desktop does the same on a pill click.
        if (!reacted && isPositiveEmojiParticle(reaction.emoji)) {
          burstEmojiFromContext(ref, context, reaction.emoji);
        }
        onTap();
      },
      onLongPress: onLongPress,
      child: _PillSurface(
        highlighted: reacted,
        minWidth: _pillMinWidth,
        child: Row(
          mainAxisSize: MainAxisSize.min,
          // Centered so a short reaction sits mid-pill once the min width kicks
          // in rather than hugging the left edge.
          mainAxisAlignment: MainAxisAlignment.center,
          children: [
            _ReactionEmoji(reaction: reaction, size: _pillGlyphSize),
            const SizedBox(width: _pillGap),
            // Desktop shows the count even at 1; hiding it made a fresh
            // reaction jump in width the moment a second person joined.
            Text(
              '${reaction.count}',
              style: reactionCountTextStyle.copyWith(
                color: reacted
                    ? context.colors.primary
                    : context.colors.onSurfaceVariant,
              ),
            ),
          ],
        ),
      ),
    );
  }
}

class _AddReactionPill extends StatelessWidget {
  final VoidCallback onTap;

  const _AddReactionPill({required this.onTap});

  @override
  Widget build(BuildContext context) {
    return Semantics(
      button: true,
      label: 'Add reaction',
      child: GestureDetector(
        key: const ValueKey('add-reaction-pill'),
        onTap: onTap,
        child: _PillSurface(
          highlighted: false,
          minWidth: _addPillWidth,
          child: Center(
            widthFactor: 1,
            child: Icon(
              LucideIcons.smilePlus,
              size: _pillGlyphSize,
              color: context.colors.onSurfaceVariant,
            ),
          ),
        ),
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
      return NativeEmojiGlyph(emoji: reaction.emoji, size: size);
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
  showBuzzModalBottomSheet<void>(
    context: context,
    isScrollControlled: true,
    showDragHandle: true,
    backgroundColor: context.colors.surfaceContainerHighest,
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
              padding: const EdgeInsets.symmetric(horizontal: Grid.gutter),
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
              horizontal: Grid.gutter,
              vertical: Grid.half,
            ),
            child: Row(
              children: [
                _ReactionEmoji(reaction: currentReaction, size: 32),
                const SizedBox(width: Grid.half),
                Text(
                  // Resolved from the shared emoji-mart dataset, so this name
                  // matches desktop's `emojiDisplayName` for the whole set
                  // rather than the 28 glyphs a hardcoded map used to cover.
                  ref
                      .watch(emojiDatasetOrEmptyProvider)
                      .displayName(currentReaction.emoji),
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

class _ReactorAvatar extends StatelessWidget {
  final String? avatarUrl;
  final String initial;

  const _ReactorAvatar({required this.avatarUrl, required this.initial});

  @override
  Widget build(BuildContext context) {
    return AvatarImage(
      imageUrl: avatarUrl,
      radius: 20,
      fallback: Text(initial),
    );
  }
}
