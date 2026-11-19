import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/profile/user_cache_provider.dart';
import '../../shared/theme/theme.dart';
import '../../shared/theme/buzz_icons.dart';
import '../../shared/widgets/avatar_image.dart';
import 'channel_identity_names_provider.dart';
import 'timeline_message.dart';
import 'channel_typing_provider.dart';
import 'agent_activity/inline_live_activity.dart';
import 'typing_dots.dart';
import 'activity_disclosure_state.dart';
import 'channel_management_provider.dart';
import 'message_author_visibility.dart';

/// A stable inline disclosure for quiet agent messages, including saved history.
class InlineAgentActivity extends HookConsumerWidget {
  const InlineAgentActivity({
    super.key,
    required this.channelId,
    required this.messages,
    required this.messageBuilder,
    this.working = false,
    this.liveEntries = const [],
    this.inheritLiveExpansion = false,
    this.revealMessageId,
    this.onToggle,
  });

  final String channelId;
  final List<TimelineMessage> messages;
  final Widget Function(TimelineMessage) messageBuilder;
  final bool working;
  final List<TypingEntry> liveEntries;
  final bool inheritLiveExpansion;
  final String? revealMessageId;
  final VoidCallback? onToggle;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final containsTarget = messages.any((m) => m.id == revealMessageId);
    final scope = (
      channel: channelId,
      author: messages.first.pubkey.toLowerCase(),
      root: messages.first.rootId,
    );
    final pending = ref.read(pendingActivityExpansionProvider(scope));
    final inherited = useRef(inheritLiveExpansion && pending);
    final expanded = useState(containsTarget || inherited.value);
    useEffect(() {
      if (inherited.value) {
        WidgetsBinding.instance.addPostFrameCallback((_) {
          if (context.mounted) {
            ref
                .read(pendingActivityExpansionProvider(scope).notifier)
                .setExpanded(false);
          }
        });
      }
      return null;
    }, const []);
    useEffect(() {
      if (containsTarget) expanded.value = true;
      return null;
    }, [containsTarget, revealMessageId]);
    final author = messages.first.pubkey.toLowerCase();
    final profile = ref.watch(
      userCacheProvider.select((cache) => cache[author]),
    );
    final name = watchChannelIdentityLabel(ref, channelId, author);
    final participants = messages.map((m) => m.pubkey.toLowerCase()).toSet();
    final showIdentity =
        participants.length != 1 ||
        showMessageAuthorName(
          ref,
          channelId,
          author,
          ref.watch(currentPubkeyProvider),
        );
    final title = participants.length == 1
        ? name
        : '$name and ${participants.length - 1} others';
    void toggle() {
      onToggle?.call();
      expanded.value = !expanded.value;
    }

    return Padding(
      padding: const EdgeInsets.symmetric(vertical: Grid.xs),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          if (showIdentity)
            ExcludeSemantics(
              child: AvatarImage(
                imageUrl: profile?.avatarUrl,
                radius: messageAvatarSize / 2,
                isAgent: true,
                fallback: Text(profile?.initial ?? name.substring(0, 1)),
              ),
            ),
          if (showIdentity) const SizedBox(width: messageAvatarContentGap),
          Expanded(
            child: ActivityMessageWidth(
              showAvatar: showIdentity,
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(title, style: messageUsernameTextStyle),
                  const SizedBox(height: Grid.xxs),
                  Material(
                    color: context.colors.secondaryContainer,
                    borderRadius: BorderRadius.circular(Radii.container),
                    clipBehavior: Clip.antiAlias,
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      mainAxisSize: MainAxisSize.min,
                      children: [
                        Semantics(
                          container: true,
                          button: true,
                          expanded: expanded.value,
                          label:
                              '${expanded.value ? 'Collapse' : 'Show'} activity from $title',
                          value:
                              '${messages.length} updates${working ? ', working' : ''}',
                          onTap: toggle,
                          child: ExcludeSemantics(
                            child: InkWell(
                              onTap: toggle,
                              child: ConstrainedBox(
                                constraints: const BoxConstraints(
                                  minHeight: 48,
                                  minWidth: 48,
                                ),
                                child: Padding(
                                  padding: const EdgeInsets.symmetric(
                                    horizontal: Grid.md,
                                    vertical: Grid.xs,
                                  ),
                                  child: Row(
                                    mainAxisSize: MainAxisSize.min,
                                    children: [
                                      if (!expanded.value && working)
                                        const TypingDots()
                                      else
                                        Flexible(
                                          child: Text(
                                            expanded.value
                                                ? 'Collapse'
                                                : 'Activity · ${messages.length}',
                                          ),
                                        ),
                                      const SizedBox(width: Grid.xs),
                                      Icon(
                                        expanded.value
                                            ? BuzzIcons.chevronUp
                                            : BuzzIcons.chevronDown,
                                        size: 18,
                                      ),
                                    ],
                                  ),
                                ),
                              ),
                            ),
                          ),
                        ),
                        _activityExpansion(
                          context,
                          child: expanded.value
                              ? Padding(
                                  padding: const EdgeInsets.all(Grid.xs),
                                  child: Column(
                                    crossAxisAlignment:
                                        CrossAxisAlignment.start,
                                    children: [
                                      for (final message in messages)
                                        messageBuilder(message),
                                      for (final author
                                          in liveEntries
                                              .map(
                                                (entry) =>
                                                    entry.pubkey.toLowerCase(),
                                              )
                                              .toSet())
                                        InlineLiveActivity(
                                          channelId: channelId,
                                          agentPubkey: author,
                                          threadRoots: {
                                            for (final entry in liveEntries)
                                              if (entry.pubkey.toLowerCase() ==
                                                  author)
                                                entry.threadHeadId,
                                          },
                                        ),
                                    ],
                                  ),
                                )
                              : const SizedBox.shrink(),
                        ),
                      ],
                    ),
                  ),
                ],
              ),
            ),
          ),
        ],
      ),
    );
  }
}

Widget _activityExpansion(BuildContext context, {required Widget child}) =>
    MediaQuery.disableAnimationsOf(context)
    ? child
    : AnimatedSize(
        duration: const Duration(milliseconds: 180),
        curve: Curves.easeOutCubic,
        alignment: Alignment.topLeft,
        child: child,
      );
