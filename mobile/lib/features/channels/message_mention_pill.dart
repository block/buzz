import 'package:flutter/material.dart';
import 'package:gpt_markdown/gpt_markdown.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/mentions/mention_identity_links.dart';
import '../../shared/profile/user_cache_provider.dart';
import '../../shared/relay/relay_provider.dart';
import '../../shared/widgets/buzz_agent_icon.dart';

import '../../shared/theme/theme.dart';
import 'channel_identity_names_provider.dart';

/// The inline identity label shared by chat mentions and channel actions.
class MessageMentionPill extends ConsumerWidget {
  /// Visible contextual identity name.
  final String label;

  /// Full identity label for assistive technologies.
  final String? semanticsLabel;

  /// Whether to use the desktop robot icon instead of the @ prefix.
  final bool isAgent;

  /// Resolved identity; unresolved names never receive the self highlight.
  final String? pubkey;

  /// Whether the chip sits inside the viewer’s colored message bubble.
  final bool ownBubble;

  /// Typography inherited from the surrounding message.
  final TextStyle? textStyle;

  /// Creates a chat mention with an agent icon or human @ prefix.
  const MessageMentionPill({
    super.key,
    required this.label,
    this.semanticsLabel,
    required this.isAgent,
    this.textStyle,
    this.pubkey,
    this.ownBubble = false,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final viewer = pubkey == null ? null : ref.watch(myPubkeyProvider);
    final isSelf =
        viewer != null && pubkey?.toLowerCase() == viewer.toLowerCase();
    final chat = ChatColors.of(context);
    final ink = ownBubble || isSelf
        ? chat.foreground
        : context.colors.onSurface;
    final fill = ownBubble
        ? chatCodeTint(chat.foreground, 0.18)
        : isSelf
        ? chat.outgoing
        : (context.colors.brightness == Brightness.dark
              ? Colors.white.withValues(alpha: 0.16)
              : Colors.black.withValues(alpha: 0.10));
    final style =
        (textStyle ?? context.textTheme.bodyMedium)?.copyWith(
          color: ink,
          fontWeight: FontWeight.w500,
          height: 1,
        ) ??
        TextStyle(color: ink, fontWeight: FontWeight.w500, height: 1);
    final fontSize = style.fontSize ?? 16;

    return Container(
      padding: const EdgeInsets.symmetric(
        horizontal: Grid.quarter * 1.5,
        vertical: Grid.quarter,
      ),
      decoration: BoxDecoration(
        color: fill,
        borderRadius: BorderRadius.circular(Radii.full),
      ),
      child: Row(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.center,
        children: [
          if (isAgent)
            BuzzAgentIcon(size: fontSize, color: ink)
          else
            Transform.translate(
              offset: const Offset(0, -Grid.quarter),
              child: Text('@', style: style),
            ),
          Flexible(
            child: Text(label, style: style, semanticsLabel: semanticsLabel),
          ),
        ],
      ),
    );
  }
}

/// [label] as a pill shows it: a full-key namesake qualifier, `(<64 hex>)`,
/// shortened to its first eight and last four digits.
String mentionVisibleLabel(String label) => label.replaceAllMapped(
  RegExp(r'\(([0-9a-f]{64})\)'),
  (m) => '(${m[1]!.substring(0, 8)}…${m[1]!.substring(60)})',
);

/// The pill for an identity link, `[@Name](nostr:npub1…)`: the mention an
/// edit writes into the body. The link names its own key, so it needs no
/// tag and opens that person. Like buzz-app, it shows the name that key
/// resolves to, from [mentionLabels] then [mentionNames], so a label written
/// into the link can't pass one person off as another; the written label
/// shows only when the key is unknown here. Under an [IdentityLinkChannel]
/// the pill resolves its key in that channel itself and requests its
/// profile, so a row looks up only the links it draws. It takes the self highlight and the
/// own-bubble tint as a tagged mention's pill does. Null for any other link.
InlineSpan? identityMentionSpan(
  LinkBuildDetails link,
  String url,
  TextStyle style,
  Map<String, String> mentionLabels,
  Map<String, String> mentionNames,
  Set<String> agentMentionPubkeys,
  void Function(String pubkey)? onMentionTap, {
  bool ownBubble = false,
}) {
  final pubkey = mentionIdentityLinkKey(url);
  if (pubkey == null) return null;
  var text = '';
  TextSpan(children: link.labelSpans).visitChildren((span) {
    if (span is TextSpan && span.text != null) {
      text += span.text!;
    }
    return true;
  });
  final label = text.replaceAll('\u00A0', ' ');
  if (!isMentionIdentityLabel(label)) return null;
  final pill = _IdentityLinkPill(
    pubkey: pubkey,
    written: label.substring(1),
    name: mentionLabels[pubkey],
    fallbackName: mentionIdentityLinkName(pubkey, mentionNames[pubkey]),
    isAgent: agentMentionPubkeys.any((key) => key.toLowerCase() == pubkey),
    ownBubble: ownBubble,
    style: style,
  );
  return link.asWidgetSpan(
    onMentionTap == null
        ? pill
        : GestureDetector(onTap: () => onMentionTap(pubkey), child: pill),
  );
}

/// Names the channel that identity-link pills below it resolve their keys
/// in: each pill then watches only its own key's label and requests that
/// key's profile as it builds, so a row looks up only the links it draws.
class IdentityLinkChannel extends InheritedWidget {
  final String channelId;

  const IdentityLinkChannel({
    super.key,
    required this.channelId,
    required super.child,
  });

  static String? of(BuildContext context) => context
      .dependOnInheritedWidgetOfExactType<IdentityLinkChannel>()
      ?.channelId;

  @override
  bool updateShouldNotify(IdentityLinkChannel oldWidget) =>
      oldWidget.channelId != channelId;
}

/// An identity link's pill. Under an [IdentityLinkChannel] it names its key
/// from that channel's names; otherwise from the row's [name] (contextual
/// label) then [fallbackName] (mention name), and the written label when
/// neither is known.
class _IdentityLinkPill extends HookConsumerWidget {
  final String pubkey;
  final String written;
  final String? name;
  final String? fallbackName;
  final bool isAgent;
  final bool ownBubble;
  final TextStyle style;

  const _IdentityLinkPill({
    required this.pubkey,
    required this.written,
    required this.name,
    required this.fallbackName,
    required this.isAgent,
    required this.ownBubble,
    required this.style,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final channelId = IdentityLinkChannel.of(context);
    useEffect(() {
      if (channelId != null &&
          ref.read(userCacheProvider.notifier).needsLinkedLookup(pubkey)) {
        Future.microtask(() {
          if (context.mounted) {
            ref.read(userCacheProvider.notifier).requestLinked([pubkey]);
          }
        });
      }
      return null;
    }, [channelId, pubkey]);
    final resolved = channelId == null
        ? (name: name, isAgent: false)
        : ref.watch(
            channelIdentityNamesProvider(channelId).select(
              (names) => (
                name: names.hasName(pubkey) ? names.labelFor(pubkey) : null,
                isAgent: names.isAgent(pubkey),
              ),
            ),
          );
    final shown = resolved.name ?? fallbackName;
    return MessageMentionPill(
      label: shown == null ? written : mentionVisibleLabel(shown),
      semanticsLabel: shown,
      pubkey: pubkey,
      ownBubble: ownBubble,
      isAgent: isAgent || resolved.isAgent,
      textStyle: style,
    );
  }
}
