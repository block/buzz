part of '../compose_bar.dart';

/// Opens the sheet that edits the text of [message] in [channelId].
///
/// The sheet offers the composer's `@` chooser with the same rules, and a
/// pick inserts the same bound chip. An edit replaces prose only: the saved
/// edit event carries the tags the edit path writes today and never a `p`
/// tag for the pick, so nobody is notified from an edit (buzz-app's edit
/// rule; project recommendation 9). The message's existing mentions open
/// as chips.
void showMessageEditSheet({
  required BuildContext context,
  required TimelineMessage message,
  required String channelId,
}) {
  showBuzzModalBottomSheet<void>(
    context: context,
    isScrollControlled: true,
    showDragHandle: true,
    builder: (_) => _MessageEditSheet(message: message, channelId: channelId),
  );
}

class _MessageEditSheet extends HookConsumerWidget {
  final TimelineMessage message;
  final String channelId;

  const _MessageEditSheet({required this.message, required this.channelId});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final controller = useMemoized(_MarkdownEditingController.new);
    useEffect(() => controller.dispose, [controller]);
    final focusNode = useFocusNode();
    final isModifyingText = useRef(false);
    final lastObservedEditingValue = useRef(controller.value);
    final mentions = useListenableSelector(
      controller,
      () => controller.mentions,
    );

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
      mentions: mentions,
      members: memberCandidates,
      memberKeys: [
        membersAsync.asData?.value,
        cachedMembers,
        userCache,
        agentOwners,
      ],
    );

    // The message's existing mentions, as the bubble names them, open as
    // chips. Names can arrive after the sheet (a profile still loading), so
    // the chips are brought up to date while the text is still the original.
    final seeded = _editedMessageMentions(
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
    );
    final seededKey = [
      for (final m in seeded) '${m.start}:${m.end}:${m.pubkey}',
    ].join(',');
    useEffect(() {
      final text = message.content;
      if (controller.text.isNotEmpty && controller.text != text) return null;
      final bound = [for (final m in controller.mentions) (m.range, m.pubkey)];
      if (controller.text == text &&
          listEquals(bound, [for (final m in seeded) (m.range, m.pubkey)])) {
        return null;
      }
      isModifyingText.value = true;
      try {
        controller.replaceDraft(
          TextEditingValue(
            text: text,
            selection: controller.selection.isValid
                ? controller.selection
                : TextSelection.collapsed(offset: text.length),
          ),
          mentions: seeded,
        );
      } finally {
        isModifyingText.value = false;
      }
      return null;
    }, [controller, seededKey]);

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
      final text = _qualifiedMentionContent(
        draft,
        controller.mentions,
        code: controller._styledCodeRanges(draft),
      ).trim();
      if (text.isEmpty || text == message.content) {
        Navigator.of(context).pop();
        return;
      }
      // Prose only: the edit keeps the tags this path always wrote and adds
      // no recipient for a pick.
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

    return Padding(
      padding: EdgeInsets.fromLTRB(
        Grid.gutter,
        0,
        Grid.gutter,
        MediaQuery.viewInsetsOf(context).bottom,
      ),
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

/// The mentions already in [message], bound to the people its tags name,
/// in text order. Each `@Name` or `@Name (<key>)` the content writes for
/// exactly one tagged person becomes a chip; a name two tagged people share
/// stays text, since the text cannot say which one it meant.
List<_ComposerMention> _editedMessageMentions(
  TimelineMessage message, {
  required Map<String, String> names,
  required Set<String> agentPubkeys,
  required List<MentionCandidate> members,
}) {
  if (names.isEmpty) return const [];
  final keysByLabel = <String, Set<String>>{};
  for (final MapEntry(key: pubkey, value: name) in names.entries) {
    (keysByLabel[name.toLowerCase()] ??= {}).add(pubkey);
    keysByLabel['${name.toLowerCase()} ($pubkey)'] = {pubkey};
  }
  final content = message.content;
  return [
    for (final range in mentionOccurrences(content, keysByLabel.keys))
      if (keysByLabel[range.label.toLowerCase()] case final keys?
          when keys.length == 1)
        _ComposerMention(
          start: range.start,
          end: range.end,
          candidate:
              members
                  .where((m) => m.pubkey.toLowerCase() == keys.single)
                  .firstOrNull ??
              MentionCandidate(
                pubkey: keys.single,
                displayName: names[keys.single],
                isAgent: agentPubkeys.contains(keys.single),
              ),
        ),
  ];
}
