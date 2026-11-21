part of '../compose_bar.dart';

/// Opens the sheet that edits the text of [message] in [channelId].
///
/// The sheet offers the composer's `@` chooser with the same rules, and a
/// pick inserts the same bound chip. An edit notifies nobody: the saved
/// edit event carries the tags the edit path writes today and never a `p`
/// tag. Each chip is saved in the body as an identity link to its exact key
/// (`[@Name](nostr:npub1…)`, buzz-app's edit rule; project recommendation
/// 9), which the bubble draws as that person's pill. The message's existing
/// mentions and identity links open as chips, except a name inside code or
/// inside another Markdown link, which stays the text it was; a pick made
/// there stays text too, so no link is ever written inside a link.
void showMessageEditSheet({
  required BuildContext context,
  required TimelineMessage message,
  required String channelId,
}) {
  // One instance for the sheet's life: the route calls its builder again on
  // every keyboard frame, and an identical widget is not rebuilt.
  final sheet = _MessageEditSheet(message: message, channelId: channelId);
  showBuzzModalBottomSheet<void>(
    context: context,
    isScrollControlled: true,
    showDragHandle: true,
    builder: (_) => sheet,
  );
}

class _MessageEditSheet extends HookConsumerWidget {
  final TimelineMessage message;
  final String channelId;

  const _MessageEditSheet({required this.message, required this.channelId});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final controller = useMemoized(
      () => _MarkdownEditingController(portableProse: true),
    );
    useEffect(() => controller.dispose, [controller]);
    final focusNode = useFocusNode();
    final stylusHandwriting = useMemoized(
      () => StylusHandwritingCutoff(controller, focusNode),
      [controller, focusNode],
    );
    useEffect(() => stylusHandwriting.dispose, [stylusHandwriting]);
    final isModifyingText = useRef(false);
    final lastObservedEditingValue = useRef(controller.value);

    final currentPubkey = ref.watch(currentPubkeyProvider);
    final userCache = ref.watch(userCacheProvider);
    final channelsAsync = ref.watch(channelsProvider);
    final membersAsync = ref.watch(channelMembersProvider(channelId));
    final sessionStatus = ref.watch(relaySessionProvider).status;
    final cachedMembers = channelsAsync.asData == null
        ? const <ChannelMember>[]
        : ref
              .read(channelsProvider.notifier)
              .cachedMembersForChannel(channelId);
    final agentOwners = ref.watch(agentOwnersProvider).asData?.value;
    final channel = ref.watch(channelByIdProvider(channelId));
    List<MentionCandidate> memberCandidates() => _typedMemberCandidates(
      membersAsync: membersAsync,
      sessionStatus: sessionStatus,
      cachedMembers: cachedMembers,
      userCache: userCache,
      agentOwners: agentOwners,
    );
    _useMentionChipQualifiers(
      ref: ref,
      controller: controller,
      roster: channelMembersForAutocomplete(
        membersAsync: membersAsync,
        sessionStatus: sessionStatus,
        cachedMembers: cachedMembers,
      ),
      userCache: userCache,
      agentOwners: agentOwners,
    );

    // The message's existing mentions, as the bubble names them, open as
    // chips. The field is filled once. Names can arrive after the sheet (a
    // profile still loading), so the chips are brought up to date only
    // while the draft is still exactly what was last seeded: a field the
    // user cleared or changed is never filled again.
    final parsed = _editedMessageDraft(
      message,
      names: _editedMentionNames(
        ref,
        message: message,
        channelId: channelId,
        userCache: userCache,
        members: channelMembersForAutocomplete(
          membersAsync: membersAsync,
          sessionStatus: sessionStatus,
          cachedMembers: cachedMembers,
        ),
      ),
      agentPubkeys: ref.watch(agentMentionPubkeysProvider(channelId)),
      members: memberCandidates(),
      code: controller._styledCodeRanges(message.content),
    );
    // A chip is seeded only where the bubble draws a pill and every other
    // Markdown reader reads prose. A name inside code (a span or fence of
    // any form, or an indented block) or a link (`[@Bob](https://…)`, a
    // reference `[@Bob]` and its definition in any case) is that code's or
    // link's own text: it opens as text, so saving leaves it exactly as
    // written.
    final seed = (
      text: parsed.text,
      mentions: controller._inProse(parsed.text, parsed.mentions),
    );
    final seeded = useRef<_EditedDraft?>(null);
    _EditedDraft currentDraft() => (
      text: controller.text,
      bound: [for (final m in controller.mentions) (m.range, m.pubkey)],
    );
    final seedBound = [for (final m in seed.mentions) (m.range, m.pubkey)];
    useEffect(() {
      final last = seeded.value;
      if (last != null &&
          (!_sameEditedDraft(last, currentDraft()) ||
              listEquals(last.bound, seedBound))) {
        return null;
      }
      isModifyingText.value = true;
      try {
        controller.replaceDraft(
          TextEditingValue(
            text: seed.text,
            selection: last != null && controller.selection.isValid
                ? controller.selection
                : TextSelection.collapsed(offset: seed.text.length),
          ),
          mentions: seed.mentions,
        );
      } finally {
        isModifyingText.value = false;
      }
      seeded.value = (text: seed.text, bound: seedBound);
      return null;
    }, [controller, seedBound.join(',')]);
    // Read after seeding, so a long message opens with handwriting already off
    // and the first frame never measures it.
    final stylusHandwritingEnabled = useValueListenable(stylusHandwriting);

    _usePlatformUndoBindings(controller, focusNode);
    final chooser = _useMentionChooser(
      ref,
      channelId: channelId,
      controller: controller,
      focusNode: focusNode,
      isModifyingText: isModifyingText,
      viewer: currentPubkey,
    );
    useEffect(() {
      lastObservedEditingValue.value = controller.value;
      void listener() {
        final editingValue = controller.value;
        final previousValue = lastObservedEditingValue.value;
        lastObservedEditingValue.value = editingValue;
        if (isModifyingText.value || editingValue == previousValue) return;
        chooser.track(previousValue, editingValue);
      }

      controller.addListener(listener);
      return () => controller.removeListener(listener);
    }, [controller]);

    // The chooser belongs to the focused field (recommendation 8).
    final chooserOpen = useListenable(focusNode).hasFocus;
    final rows = chooser.rows;
    final suggestions = chooserOpen ? rows.rows : const <MentionCandidate>[];
    final searchFailed = chooserOpen && rows.searchFailed;
    final pickerVisible = suggestions.isNotEmpty || searchFailed;
    final topMention = suggestions
        .where((s) => !rows.unavailable.contains(s.pubkey))
        .firstOrNull;
    final pickerQuery = (chooser.query, null);
    KeyEventResult handleHardwareKey(FocusNode _, KeyEvent event) =>
        _handleComposerHardwareKey(
          event,
          controller,
          inputOwned: false,
          query: (chooser.liveQuery.value, null),
          renderedQuery: pickerQuery,
          pickerVisible: pickerVisible,
          acceptSuggestion: topMention != null
              ? () => _runComposerAction(() => chooser.insert(topMention))
              : null,
          send: null,
        );

    void save() {
      final draft = controller.text;
      final text = _editedMentionContent(
        draft,
        controller.mentions,
        code: controller._styledCodeRanges(draft),
      ).trim();
      final unchanged =
          seeded.value != null &&
          _sameEditedDraft(seeded.value!, currentDraft());
      if (text.isEmpty || unchanged || text == message.content) {
        Navigator.of(context).pop();
        return;
      }
      // The edit keeps the tags this path always wrote and adds no
      // recipient: every chip is in the body as an identity link.
      ref
          .read(channelActionsProvider)
          .editMessage(
            channelId: channelId,
            eventId: message.id,
            content: text,
            mediaTags: buildCustomEmojiTags(
              text,
              ref.read(customEmojiListProvider),
            ),
          );
      Navigator.of(context).pop();
    }

    return _KeyboardInset(
      child: SingleChildScrollView(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            if (pickerVisible)
              Padding(
                padding: const EdgeInsets.only(bottom: Grid.xxs),
                child: _composerSuggestionPanel(
                  channelSuggestions: const [],
                  mentionSuggestions: suggestions,
                  unavailableMentions: rows.unavailable,
                  mentionSearchFailed: searchFailed,
                  reserveMentionHeight: rows.reserveHeight,
                  mentionSearching: rows.searching,
                  onMentionSearchRetry: () {
                    if (chooser.query case final query?) {
                      ref.invalidate(mentionUserSearchProvider(query));
                    }
                  },
                  userCache: userCache,
                  currentPubkey: currentPubkey,
                  isDmChannel: channel?.isDm ?? false,
                  onChannelSelect: (_) {},
                  onMentionSelect: chooser.insert,
                ),
              ),
            Actions(
              actions: {
                UndoTextIntent: _ComposerHistoryAction<UndoTextIntent>(
                  controller,
                ),
                RedoTextIntent: _ComposerHistoryAction<RedoTextIntent>(
                  controller,
                ),
              },
              child: Focus(
                canRequestFocus: false,
                skipTraversal: true,
                onKeyEvent: handleHardwareKey,
                child: TextField(
                  key: const ValueKey('message-edit-field'),
                  controller: controller,
                  focusNode: focusNode,
                  autofocus: true,
                  minLines: 1,
                  maxLines: 5,
                  stylusHandwritingEnabled: stylusHandwritingEnabled,
                  inputFormatters: [_ComposerChipEditingFormatter(controller)],
                  decoration: const InputDecoration(hintText: 'Edit message'),
                ),
              ),
            ),
            const SizedBox(height: Grid.xxs),
            Row(
              mainAxisAlignment: MainAxisAlignment.end,
              children: [
                TextButton(
                  onPressed: () => Navigator.of(context).pop(),
                  child: const Text('Cancel'),
                ),
                const SizedBox(width: Grid.half),
                FilledButton(onPressed: save, child: const Text('Save')),
              ],
            ),
          ],
        ),
      ),
    );
  }
}

/// Names for the people [message] mentions, as the bubble resolves them: the
/// cached profile, the roster's display name, the agent directory, then an
/// agent's fallback label.
Map<String, String> _editedMentionNames(
  WidgetRef ref, {
  required TimelineMessage message,
  required String channelId,
  required Map<String, UserProfile> userCache,
  required List<ChannelMember> members,
}) {
  final profileNames = <String, String>{};
  for (final pubkey in message.mentionPubkeys) {
    final key = pubkey.toLowerCase();
    final name =
        userCache[key]?.displayName ??
        members
            .where((m) => m.pubkey.toLowerCase() == key)
            .firstOrNull
            ?.displayName;
    if (name != null && name.trim().isNotEmpty) profileNames[key] = name;
  }
  final agentPubkeys = ref.watch(agentMentionPubkeysProvider(channelId));
  return mentionNamesWithDirectoryLabels(
    mentionPubkeys: message.mentionPubkeys,
    profileMentionNames: profileNames,
    directoryDisplayNames: ref.watch(agentDirectoryDisplayNamesProvider),
    agentMentionPubkeys: {
      for (final pubkey in message.mentionPubkeys)
        if (agentPubkeys.contains(pubkey.toLowerCase())) pubkey.toLowerCase(),
    },
  );
}

/// Keeps its [child] above the keyboard. The inset changes on every frame
/// of the keyboard's animation; reading it here rebuilds this padding alone,
/// not the sheet and the roster's mention candidates it builds.
class _KeyboardInset extends StatelessWidget {
  final Widget child;

  const _KeyboardInset({required this.child});

  @override
  Widget build(BuildContext context) => Padding(
    padding: EdgeInsets.fromLTRB(
      Grid.gutter,
      0,
      Grid.gutter,
      MediaQuery.viewInsetsOf(context).bottom,
    ),
    child: child,
  );
}

/// An edit draft's text and the key bound at each chip's range.
typedef _EditedDraft = ({String text, List<(MentionRange, String)> bound});

bool _sameEditedDraft(_EditedDraft a, _EditedDraft b) =>
    a.text == b.text && listEquals(a.bound, b.bound);

final _keyQualifier = RegExp(r' \(([0-9a-f]{64})\)$', caseSensitive: false);

/// The body an edit saves for [text]: each chip becomes an identity link to
/// its exact key, a display-only reference that notifies nobody and that
/// every reader resolves without a tag. Two picked namesakes are two links
/// with the same label, so no key is written as prose. A chip inside [code]
/// the composer styles stays as typed.
String _editedMentionContent(
  String text,
  List<_ComposerMention> mentions, {
  required List<TextRange> code,
}) {
  var content = text;
  for (final mention in mentions.reversed) {
    if (code.any((c) => c.start < mention.start && mention.end < c.end)) {
      continue;
    }
    // A chip seeded from `@Name (<key>)` names its key in the link instead.
    var label = text.substring(mention.start, mention.end);
    if (_keyQualifier.firstMatch(label)?[1]?.toLowerCase() == mention.pubkey) {
      label = label.replaceFirst(_keyQualifier, '');
    }
    content = content.replaceRange(
      mention.start,
      mention.end,
      mentionIdentityLink(label, mention.pubkey),
    );
  }
  return content;
}

/// The draft that edits [message]: its text, with each identity link the
/// bubble draws as a pill (outside [code] and all other code and links)
/// shown as its `@Name`, and its chips in text order. A chip is an
/// identity link, bound to the key it names, or an `@Name` or
/// `@Name (<key>)` the content writes for exactly one tagged person; a name
/// two tagged people share stays text, since the text cannot say which one
/// it meant.
({String text, List<_ComposerMention> mentions}) _editedMessageDraft(
  TimelineMessage message, {
  required Map<String, String> names,
  required Set<String> agentPubkeys,
  required List<MentionCandidate> members,
  required List<TextRange> code,
}) {
  _ComposerMention chip(int start, int end, String pubkey, String? name) =>
      _ComposerMention(
        start: start,
        end: end,
        candidate:
            members
                .where((m) => m.pubkey.toLowerCase() == pubkey)
                .firstOrNull ??
            MentionCandidate(
              pubkey: pubkey,
              displayName: name,
              isAgent: agentPubkeys.contains(pubkey),
            ),
      );

  final text = StringBuffer();
  final mentions = <_ComposerMention>[];
  var offset = 0;
  final links = mentionIdentityLinks(message.content).toList();
  // An identity link the bubble shows as raw text (inside code of any kind)
  // or inside another link stays as written.
  final literal = mentionsInsideMarkdownLinks(
    message.content,
    [for (final link in links) (start: link.start, end: link.end)],
    keep: 0,
    portable: true,
  );
  for (final (i, link) in links.indexed) {
    if (literal.contains(i) ||
        code.any((c) => c.start < link.start && link.end < c.end)) {
      continue;
    }
    text.write(message.content.substring(offset, link.start));
    final start = text.length;
    text.write(link.label);
    mentions.add(
      chip(start, text.length, link.pubkey, link.label.substring(1)),
    );
    offset = link.end;
  }
  text.write(message.content.substring(offset));
  final draft = text.toString();

  final keysByLabel = <String, Set<String>>{};
  for (final MapEntry(key: pubkey, value: name) in names.entries) {
    // Folded as the scan compares names, so `Sam` and `ſam` share one entry
    // and an ambiguous historical label binds no key.
    (keysByLabel[foldMentionLabel(name)] ??= {}).add(pubkey);
    keysByLabel[foldMentionLabel('$name ($pubkey)')] = {pubkey};
  }
  final linked = [...mentions];
  for (final range in mentionOccurrences(draft, keysByLabel.keys)) {
    if (linked.any((m) => m.start < range.end && range.start < m.end)) continue;
    if (keysByLabel[foldMentionLabel(range.label)] case final keys?
        when keys.length == 1) {
      mentions.add(
        chip(range.start, range.end, keys.single, names[keys.single]),
      );
    }
  }
  return (
    text: draft,
    mentions: mentions..sort((a, b) => a.start.compareTo(b.start)),
  );
}
