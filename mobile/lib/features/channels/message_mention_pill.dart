import 'package:flutter/material.dart';
import 'package:gpt_markdown/gpt_markdown.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/mentions/mention_identity_links.dart';
import '../../shared/relay/relay_provider.dart';
import '../../shared/widgets/buzz_agent_icon.dart';

import '../../shared/theme/theme.dart';

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
/// shows only when the key is unknown here. It takes the self highlight and
/// the own-bubble tint as a tagged mention's pill does. Null for any other
/// link.
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
  final name =
      mentionLabels[pubkey] ??
      mentionIdentityLinkName(pubkey, mentionNames[pubkey]);
  final pill = MessageMentionPill(
    label: name == null ? label.substring(1) : mentionVisibleLabel(name),
    semanticsLabel: name,
    pubkey: pubkey,
    ownBubble: ownBubble,
    isAgent: agentMentionPubkeys.any((key) => key.toLowerCase() == pubkey),
    textStyle: style,
  );
  return link.asWidgetSpan(
    onMentionTap == null
        ? pill
        : GestureDetector(onTap: () => onMentionTap(pubkey), child: pill),
  );
}
