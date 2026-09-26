part of '../message_content.dart';

List<MarkdownComponent> _useMessageInlineComponents({
  required String content,
  required String finalContent,
  required Map<String, String> mentionNames,
  required Map<String, Set<String>> bindings,
  required Set<String> agentPubkeys,
  required Map<String, String> channelNames,
  required List<CustomEmoji> customEmoji,
  required double emojiSize,
  required List<List<String>> tags,
  required bool hasMediaReply,
  required bool hasMediaMore,
  required void Function(String)? onMentionTap,
  required void Function(String) onChannelTap,
}) {
  // Parsed spans retain these callbacks. Forward to the current handlers without
  // invalidating the parser for a new closure from an otherwise unchanged row.
  final mentionHandler = useRef(onMentionTap)..value = onMentionTap;
  final channelHandler = useRef(onChannelTap)..value = onChannelTap;
  final mentionTap = useMemoized(
    () =>
        (String pubkey) => mentionHandler.value?.call(pubkey),
    const [],
  );
  final channelTap = useMemoized(
    () =>
        (String channelId) => channelHandler.value(channelId),
    const [],
  );
  final inputs = _InlineComponentInputs(
    content: content,
    finalContent: finalContent,
    mentionNames: mentionNames,
    bindings: bindings,
    agentPubkeys: agentPubkeys,
    channelNames: channelNames,
    customEmoji: customEmoji,
    emojiSize: emojiSize,
    tags: tags,
    hasMediaReply: hasMediaReply,
    hasMediaMore: hasMediaMore,
    hasMentionHandler: onMentionTap != null,
  );

  // gpt_markdown compares components by identity. Recreating them on every
  // parent rebuild reparses unchanged message bodies on the UI isolate.
  return useMemoized(
    () => [
      _MentionMd(
        mentionNames: inputs.mentionNames,
        bindings: inputs.bindings,
        displayLabels: {
          for (final range in mentionOccurrences(content, bindings.keys))
            range.label: content.substring(range.start + 1, range.end),
        },
        agentMentionPubkeys: inputs.agentPubkeys,
        onMentionTap: inputs.hasMentionHandler ? mentionTap : null,
      ),
      CustomEmojiMd(inputs.customEmoji, content: finalContent, size: emojiSize),
      _ChannelLinkMd(
        channelNames: inputs.channelNames,
        onChannelTap: channelTap,
      ),
      ...MarkdownComponent.inlineComponents,
    ],
    [inputs],
  );
}

class _InlineComponentInputs {
  final String content;
  final String finalContent;
  final Map<String, String> mentionNames;
  final Map<String, Set<String>> bindings;
  final Set<String> agentPubkeys;
  final Map<String, String> channelNames;
  final List<CustomEmoji> customEmoji;
  final double emojiSize;
  final bool hasMentionHandler;
  final List<List<String>> tags;
  final bool hasMediaReply;
  final bool hasMediaMore;

  _InlineComponentInputs({
    required this.content,
    required this.finalContent,
    required Map<String, String> mentionNames,
    required Map<String, Set<String>> bindings,
    required Set<String> agentPubkeys,
    required Map<String, String> channelNames,
    required List<CustomEmoji> customEmoji,
    required this.emojiSize,
    required this.hasMentionHandler,
    required List<List<String>> tags,
    required this.hasMediaReply,
    required this.hasMediaMore,
  }) : tags = List.unmodifiable(
         tags.map((tag) => List<String>.unmodifiable(tag)),
       ),
       mentionNames = Map.unmodifiable(mentionNames),
       bindings = Map.unmodifiable({
         for (final entry in bindings.entries)
           entry.key: Set<String>.unmodifiable(entry.value),
       }),
       agentPubkeys = Set.unmodifiable(agentPubkeys),
       channelNames = Map.unmodifiable(channelNames),
       customEmoji = List.unmodifiable(customEmoji);

  @override
  bool operator ==(Object other) =>
      other is _InlineComponentInputs &&
      content == other.content &&
      finalContent == other.finalContent &&
      emojiSize == other.emojiSize &&
      hasMentionHandler == other.hasMentionHandler &&
      hasMediaReply == other.hasMediaReply &&
      hasMediaMore == other.hasMediaMore &&
      tags.length == other.tags.length &&
      tags.indexed.every(
        (entry) => listEquals(entry.$2, other.tags[entry.$1]),
      ) &&
      mapEquals(mentionNames, other.mentionNames) &&
      mapEquals(channelNames, other.channelNames) &&
      setEquals(agentPubkeys, other.agentPubkeys) &&
      listEquals(customEmoji, other.customEmoji) &&
      bindings.length == other.bindings.length &&
      bindings.entries.every(
        (entry) => setEquals(entry.value, other.bindings[entry.key]),
      );

  @override
  int get hashCode =>
      Object.hash(content, finalContent, emojiSize, hasMentionHandler);
}
