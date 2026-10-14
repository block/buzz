import 'dart:async';
import 'package:flutter/foundation.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:buzz/shared/theme/buzz_icons.dart';

import '../../shared/theme/theme.dart';
import '../../shared/widgets/app_list_card_item.dart';
import '../../shared/widgets/avatar_image.dart';
import '../../shared/widgets/modal_presentation.dart';
import '../../shared/widgets/native_message_presentation.dart';
import '../../shared/relay/media_auth.dart';
import '../../shared/custom_emoji/custom_emoji_render.dart';
import '../../shared/emoji/emoji_burst.dart';
import '../../shared/emoji/emoji_avatar.dart';
import '../../shared/emoji/emoji_data_provider.dart';
import '../../shared/emoji/native_emoji_glyph.dart';
import '../../shared/emoji/positive_emoji.dart';
import '../../shared/profile/user_cache_provider.dart';
import '../../shared/profile/user_profile.dart';
import 'channel_identity_names_provider.dart';
import 'channel_management_provider.dart';
import 'emoji_picker.dart';
import 'recent_emoji_provider.dart';
import 'message_long_press_region.dart';
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
  final reaction = message.reactions.where((r) => r.emoji == emoji).firstOrNull;
  if (reaction == null) return;
  final actions = ref.read(channelActionsProvider);
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

class ReactionRow extends HookWidget {
  /// The message these reactions belong to. Used to match a pending burst to
  /// the right pill — the same emoji can be pending on a different message.
  final String messageId;

  /// Channel whose members define the reactor names' comparison context.
  final String channelId;

  /// Groups reaction controls in one capsule attached to a bubble.
  final bool combined;

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
    required this.channelId,
    required this.reactions,
    required this.onToggle,
    this.showAddButton = false,
    this.combined = false,
    this.onAddReaction,
  });

  @override
  Widget build(BuildContext context) {
    final liveReactions = useMemoized(() => ValueNotifier(reactions));
    useEffect(
      () => () {
        scheduleMicrotask(() {
          liveReactions.value = [];
          liveReactions.dispose();
        });
      },
      [liveReactions],
    );
    final latestToggle = useRef(onToggle)..value = onToggle;
    useEffect(() {
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (context.mounted) liveReactions.value = reactions;
      });
      return null;
    }, [reactions]);
    // With the add button on, an empty row still renders — that affordance is
    // the point. Without it, an empty row would be dead space.
    if (reactions.isEmpty && !showAddButton) return const SizedBox.shrink();

    if (combined && reactions.isNotEmpty) {
      final count = reactions.fold<int>(
        0,
        (sum, reaction) => sum + reaction.count,
      );
      final ownReaction = reactions.any(
        (reaction) => reaction.reactedByCurrentUser,
      );
      void openDetails() => showReactionDetailSheet(
        context: context,
        channelId: channelId,
        reactions: liveReactions,
        onToggle: (emoji) {
          if (context.mounted) latestToggle.value(emoji);
        },
        onAddReaction: showAddButton ? onAddReaction : null,
      );
      return Padding(
        padding: const EdgeInsets.only(bottom: Grid.xxs),
        child: Align(
          widthFactor: 1,
          alignment: Alignment.centerLeft,
          child: Semantics(
            button: true,
            label:
                'View $count ${count == 1 ? 'reaction' : 'reactions'}'
                '${ownReaction ? ', including yours' : ''}',
            child: GestureDetector(
              key: const ValueKey('combined-reaction-tray'),
              behavior: HitTestBehavior.opaque,
              onTap: openDetails,
              onLongPress: openDetails,
              child: ExcludeSemantics(
                child: DecoratedBox(
                  decoration: BoxDecoration(
                    color: context.colors.secondaryContainer,
                    borderRadius: BorderRadius.circular(Radii.container),
                    border: Border.all(color: context.colors.surface, width: 2),
                  ),
                  child: Padding(
                    padding: const EdgeInsets.symmetric(
                      horizontal: Grid.xxs + Grid.quarter,
                      vertical: Grid.half,
                    ),
                    child: Wrap(
                      crossAxisAlignment: WrapCrossAlignment.center,
                      spacing: Grid.quarter,
                      runSpacing: Grid.quarter,
                      children: [
                        for (final reaction in reactions)
                          Padding(
                            padding: const EdgeInsets.all(Grid.quarter),
                            child: _ReactionEmoji(reaction: reaction, size: 20),
                          ),
                        Padding(
                          padding: const EdgeInsets.only(left: Grid.half),
                          child: Text(
                            '${reactions.fold<int>(0, (sum, reaction) => sum + reaction.count)}',
                            style: context.textTheme.labelMedium,
                          ),
                        ),
                      ],
                    ),
                  ),
                ),
              ),
            ),
          ),
        ),
      );
    }

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
                channelId: channelId,
                reactions: liveReactions,
                onToggle: (emoji) {
                  if (context.mounted) latestToggle.value(emoji);
                },
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

    return Semantics(
      onLongPress: onLongPress,
      child: RawGestureDetector(
        gestures: {
          LongPressGestureRecognizer:
              GestureRecognizerFactoryWithHandlers<LongPressGestureRecognizer>(
                () => LongPressGestureRecognizer(
                  duration: defaultTargetPlatform == TargetPlatform.iOS
                      ? iosMessageLongPressDuration
                      : null,
                ),
                (recognizer) => recognizer.onLongPress = onLongPress,
              ),
        },
        child: GestureDetector(
          key: ValueKey('reaction-pill-${reaction.emoji}'),
          onTap: () {
            // The pill is already here, so burst straight away rather than waiting
            // for the relay echo — desktop does the same on a pill click.
            if (!reacted && isPositiveEmojiParticle(reaction.emoji)) {
              burstEmojiFromContext(ref, context, reaction.emoji);
            }
            onTap();
          },
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
              BuzzIcons.smilePlus,
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
      return NativeEmojiGlyph(
        emoji: reaction.emoji,
        size: size,
        opticalBoxSize: size,
        centerInOpticalBox: true,
      );
    }
    final shortcode = reaction.emoji.substring(1, reaction.emoji.length - 1);
    return SizedBox.square(
      dimension: size,
      child: FittedBox(
        fit: BoxFit.scaleDown,
        child: CustomEmojiImage(
          shortcode: shortcode,
          url: emojiUrl,
          size: size,
        ),
      ),
    );
  }
}

Future<void> showReactionDetailSheet({
  required BuildContext context,
  required String channelId,
  required ValueListenable<List<TimelineReaction>> reactions,
  required ValueChanged<String> onToggle,
  VoidCallback? onAddReaction,
}) async {
  if (reactions.value.isEmpty) return;
  if (await _showNativeReactionDetails(
    context,
    channelId,
    reactions,
    onToggle,
    onAddReaction,
  )) {
    return;
  }
  if (!context.mounted) return;
  await showBuzzModalBottomSheet<void>(
    context: context,
    title: 'Reactions',
    isScrollControlled: true,
    showDragHandle: true,
    builder: (_) => ValueListenableBuilder<List<TimelineReaction>>(
      valueListenable: reactions,
      builder: (sheetContext, value, _) {
        if (value.isEmpty) {
          WidgetsBinding.instance.addPostFrameCallback((_) {
            if (sheetContext.mounted &&
                ModalRoute.of(sheetContext)?.isCurrent == true) {
              Navigator.of(sheetContext).pop();
            }
          });
          return const SizedBox.shrink();
        }
        return _ReactionDetailSheet(
          channelId: channelId,
          reactions: value,
          onToggle: onToggle,
          onAddReaction: onAddReaction,
        );
      },
    ),
  );
}

class _ReactionDetailSheet extends HookConsumerWidget {
  final String channelId;
  final List<TimelineReaction> reactions;
  final ValueChanged<String> onToggle;
  final VoidCallback? onAddReaction;

  const _ReactionDetailSheet({
    required this.channelId,
    required this.reactions,
    required this.onToggle,
    this.onAddReaction,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final userCache = ref.watch(userCacheProvider);
    final identityNames = ref.watch(channelIdentityNamesProvider(channelId));
    final dataset = ref.watch(emojiDatasetOrEmptyProvider);
    final rows = [
      for (final reaction in reactions)
        for (final pubkey in reaction.userPubkeys.toSet())
          (pubkey: pubkey, reaction: reaction),
    ];
    final pubkeys = rows.map((row) => row.pubkey).toSet().toList();
    useEffect(() {
      if (pubkeys.isNotEmpty) {
        ref.read(userCacheProvider.notifier).preload(pubkeys);
      }
      return null;
    }, [pubkeys.join(',')]);
    return ConstrainedBox(
      key: const ValueKey('reaction-details-sheet'),
      constraints: BoxConstraints(
        maxHeight: MediaQuery.sizeOf(context).height * 0.5,
      ),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          Padding(
            padding: const EdgeInsets.symmetric(horizontal: Grid.gutter),
            child: SingleChildScrollView(
              scrollDirection: Axis.horizontal,
              child: Row(
                children: [
                  if (onAddReaction != null)
                    IconButton(
                      key: const ValueKey('reaction-sheet-add'),
                      tooltip: 'Add reaction',
                      icon: const Icon(BuzzIcons.smilePlus),
                      onPressed: () {
                        Navigator.of(context).pop();
                        onAddReaction!();
                      },
                    ),
                  for (final reaction in reactions)
                    Padding(
                      padding: const EdgeInsets.only(right: Grid.half),
                      child: Builder(
                        builder: (chipContext) => FilterChip(
                          key: ValueKey('reaction-toggle-${reaction.emoji}'),
                          tooltip: dataset.displayName(reaction.emoji),
                          side: BorderSide(
                            color: reaction.reactedByCurrentUser
                                ? context.colors.primary.withValues(alpha: 0.40)
                                : context.colors.onSurface.withValues(
                                    alpha: 0.25,
                                  ),
                          ),
                          label: Row(
                            mainAxisSize: MainAxisSize.min,
                            children: [
                              _ReactionEmoji(reaction: reaction, size: 20),
                              const SizedBox(width: Grid.quarter),
                              Text('${reaction.count}'),
                            ],
                          ),
                          selected: reaction.reactedByCurrentUser,
                          onSelected: (_) {
                            if (!reaction.reactedByCurrentUser) {
                              burstEmojiFromContext(
                                ref,
                                chipContext,
                                reaction.emoji,
                              );
                            }
                            onToggle(reaction.emoji);
                          },
                        ),
                      ),
                    ),
                ],
              ),
            ),
          ),

          Flexible(
            child: ListView.builder(
              shrinkWrap: true,
              padding: EdgeInsets.only(
                left: Grid.gutter,
                right: Grid.gutter,
                top: Grid.xxs,
                bottom: MediaQuery.viewPaddingOf(context).bottom + Grid.half,
              ),
              itemCount: rows.length,
              itemBuilder: (context, index) {
                final row = rows[index];
                return AppListCardItem(
                  index: index,
                  itemCount: rows.length,
                  dividerIndent: Grid.xs + 40 + Grid.xs,
                  child: _ReactorTile(
                    profile: userCache[row.pubkey.toLowerCase()],
                    pubkey: row.pubkey,
                    displayName: identityNames.labelFor(row.pubkey),
                    reaction: row.reaction,
                    reactionLabel: dataset.displayName(row.reaction.emoji),
                    compact: defaultTargetPlatform == TargetPlatform.android,
                  ),
                );
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
  final String displayName;
  final TimelineReaction reaction;
  final String reactionLabel;
  final bool compact;

  const _ReactorTile({
    required this.profile,
    required this.pubkey,
    required this.displayName,
    required this.reaction,
    required this.reactionLabel,
    this.compact = false,
  });

  @override
  Widget build(BuildContext context) {
    final about = profile?.about;

    return ListTile(
      key: ValueKey('reactor-$pubkey-${reaction.emoji}'),
      minTileHeight: compact ? 52 : null,
      contentPadding: compact
          ? const EdgeInsets.symmetric(horizontal: Grid.gutter)
          : null,
      trailing: Semantics(
        label: reactionLabel,
        child: ExcludeSemantics(
          child: _ReactionEmoji(reaction: reaction, size: 26),
        ),
      ),
      leading: _ReactorAvatar(
        radius: compact ? 18 : 20,
        avatarUrl: profile?.avatarUrl,
        initial:
            profile?.initial ??
            (pubkey.isNotEmpty ? pubkey[0].toUpperCase() : '?'),
        isAgent: profile?.isAgent == true,
      ),
      title: Text(
        displayName,
        style: context.textTheme.bodyMedium?.copyWith(
          fontWeight: FontWeight.w600,
        ),
        overflow: TextOverflow.ellipsis,
      ),
      subtitle: !compact && about != null && about.isNotEmpty
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
  final bool isAgent;
  final double radius;

  const _ReactorAvatar({
    required this.avatarUrl,
    required this.initial,
    required this.isAgent,
    required this.radius,
  });

  @override
  Widget build(BuildContext context) {
    return AvatarImage(
      imageUrl: avatarUrl,
      radius: radius,
      fallback: Text(initial),
      isAgent: isAgent,
    );
  }
}

Future<bool> _showNativeReactionDetails(
  BuildContext context,
  String channelId,
  ValueListenable<List<TimelineReaction>> reactions,
  ValueChanged<String> onToggle,
  VoidCallback? onAddReaction,
) async {
  if (!NativeMessagePresentation.isSupportedPlatform) return false;
  var addRequested = false;
  final handled = await NativeMessagePresentation.withLease(() async {
    final container = ProviderScope.containerOf(context, listen: false);
    final requestId = DateTime.now().microsecondsSinceEpoch.toString();
    List<String> pubkeys() =>
        reactions.value.expand((r) => r.userPubkeys).toSet().toList();
    final sent = <String, ({String name, String? avatarUrl})>{};
    Map<String, Object?> profiles({bool onlyChanges = false}) {
      if (reactions.value.isEmpty) return {};
      final cache = container.read(userCacheProvider);
      final names = container.read(channelIdentityNamesProvider(channelId));
      final result = <String, Object?>{};
      for (final pubkey in pubkeys()) {
        final value = (
          name: names.labelFor(pubkey),
          avatarUrl: cache[pubkey.toLowerCase()]?.avatarUrl,
        );
        if (onlyChanges && sent[pubkey] == value) continue;
        sent[pubkey] = value;
        final url = value.avatarUrl;
        // Buzz's emoji-avatar SVG is a small value object, not arbitrary native SVG.
        final avatar = url != null && url.length <= 8192
            ? parseEmojiAvatarDataUrl(url)
            : null;
        final validAvatar =
            avatar != null &&
            avatar.emoji.length <= 64 &&
            avatar.emoji.characters.length == 1;
        result[pubkey] = {
          'name': value.name,
          if (validAvatar) ...{
            'avatarEmoji': avatar.emoji,
            'avatarColor': avatar.colorValue,
          } else if (url != null && !url.startsWith('data:')) ...{
            'url': url,
            'headers': container
                .read(mediaGetAuthServiceProvider)
                .headersFor(url),
          },
        };
      }
      return result;
    }

    Timer? timer;
    var disposed = false;
    var flushing = false;
    var dirty = false;
    void scheduleProfiles() {
      dirty = true;
      if (disposed || timer != null || flushing) return;
      timer = Timer(const Duration(milliseconds: 50), () async {
        timer = null;
        flushing = true;
        dirty = false;
        try {
          final changes = profiles(onlyChanges: true).entries.toList();
          // Backpressure bounds each platform message; only changed users cross
          // the bridge, even when hydration publishes several thousand-user pages.
          for (
            var start = 0;
            start < changes.length && !disposed;
            start += 256
          ) {
            await NativeMessagePresentation.present('updateProfiles', {
              'requestId': requestId,
              'profiles': Map.fromEntries(changes.skip(start).take(256)),
            });
          }
        } finally {
          flushing = false;
          if (dirty && !disposed) scheduleProfiles();
        }
      });
    }

    final subscription = container.listen(
      userCacheProvider,
      (_, _) => scheduleProfiles(),
    );
    final namesSubscription = container.listen(
      channelIdentityNamesProvider(channelId),
      (_, _) => scheduleProfiles(),
    );
    final dataset = container.read(emojiDatasetOrEmptyProvider);
    List<Map<String, Object?>> reactionData() => [
      for (final reaction in reactions.value)
        {
          'emoji': reaction.emoji,
          'label': dataset.displayName(reaction.emoji),
          'count': reaction.count,
          'selected': reaction.reactedByCurrentUser,
          'burst': isPositiveEmojiParticle(reaction.emoji),
          'users': reaction.userPubkeys.toSet().toList(),
          if (reaction.emojiUrl case final String url) ...{
            'url': url,
            'headers': container
                .read(mediaGetAuthServiceProvider)
                .headersFor(url),
          },
        },
    ];
    void updateReactions() {
      if (disposed) return;
      unawaited(
        NativeMessagePresentation.present('updateReactions', {
          'requestId': requestId,
          'reactions': reactionData(),
        }),
      );
      if (reactions.value.isNotEmpty) {
        container.read(userCacheProvider.notifier).preload(pubkeys());
        scheduleProfiles();
      }
    }

    reactions.addListener(updateReactions);
    try {
      final sheetColors = utilitySurfaceThemeData(
        Theme.of(context),
      ).colorScheme;
      final pending = NativeMessagePresentation.present('reactions', {
        'requestId': requestId,
        'sheetColor': sheetColors.surface.toARGB32(),
        'foregroundColor': sheetColors.primary.toARGB32(),
        'outlineColor': Theme.of(
          context,
        ).colorScheme.onSurface.withValues(alpha: 0.25).toARGB32(),
        'dark': Theme.of(context).brightness == Brightness.dark,
        'canAddReaction': onAddReaction != null,
        'addReactionIconCodePoint': BuzzIcons.smilePlus.codePoint,
        'profiles': profiles(),
        'reactions': reactionData(),
      }, onReaction: onToggle);
      container.read(userCacheProvider.notifier).preload(pubkeys());
      final result = await pending;
      addRequested = result?['action'] == 'addReaction';
      return result != null;
    } finally {
      disposed = true;
      reactions.removeListener(updateReactions);
      timer?.cancel();
      subscription.close();
      namesSubscription.close();
    }
  });
  if (addRequested && context.mounted) onAddReaction?.call();
  return handled;
}
