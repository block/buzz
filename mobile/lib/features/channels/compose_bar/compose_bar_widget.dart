part of '../compose_bar.dart';

class ComposeBar extends HookConsumerWidget {
  final String channelId;
  final String channelName;
  final String? hintText;
  final ComposeBarOnSend onSend;

  /// Temporarily freezes composition while its destination is being changed.
  final bool interactionLocked;

  /// Intended recipients before a new Solo channel exists. Suppresses typing
  /// and membership writes; other mentions are references until creation.
  final List<ChannelMember>? draftMembers;

  /// Reports whether a send or upload is in flight.
  final ValueChanged<bool>? onSendingChanged;

  /// Lets a parent prepare its layout before the editor requests focus.
  final VoidCallback? onFocusRequested;

  /// Parent-owned if set; otherwise internally created and disposed.
  final FocusNode? focusNode;

  /// Receives a restorer which becomes a no-op after replacement/unmount.
  final ValueChanged<VoidCallback>? onFocusRestorerChanged;

  /// Optional thread IDs for thread-scoped typing indicators.
  final String? threadHeadId;
  final String? rootId;

  /// The conversation's displayed messages, oldest first; prefills the agents
  /// the viewer's latest one notified. Null turns the prefill off.
  final List<TimelineMessage>? prefillScopeMessages;
  const ComposeBar({
    super.key,
    required this.channelId,
    this.interactionLocked = false,
    this.channelName = '',
    this.hintText,
    this.draftMembers,
    this.onSendingChanged,
    this.threadHeadId,
    this.rootId,
    this.focusNode,
    this.onFocusRestorerChanged,
    this.onFocusRequested,
    this.prefillScopeMessages,
    required this.onSend,
  });
  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final sendSurfaceKey = useMemoized(GlobalKey.new);
    final controller = useMemoized(_MarkdownEditingController.new);
    final draftText = useListenableSelector(controller, () => controller.text);
    useEffect(() => controller.dispose, [controller]);
    final draftKey = composeDraftKey(channelId, threadHeadId: threadHeadId);
    final draftRevision = useRef(0);
    final draftIdentity = _composerDraftIdentity(ref);
    final isComposerExpanded = useState(false);
    final androidImeTransitionStarted = useState(
      defaultTargetPlatform != TargetPlatform.android,
    );
    final androidImeFallbackTimer = useRef<Timer?>(null);
    final ownedFocusNode = useFocusNode();
    final focusNode = this.focusNode ?? ownedFocusNode;
    final stylusHandwriting = useMemoized(
      () => StylusHandwritingCutoff(controller, focusNode),
      [controller, focusNode],
    );
    useEffect(() => stylusHandwriting.dispose, [stylusHandwriting]);
    useEffect(
      () => () {
        androidImeFallbackTimer.value?.cancel();
        _dismissComposerKeyboard(focusNode);
      },
      [focusNode],
    );
    final isEmojiPickerOpen = useState(false);
    final inputTool = useState<_InputTool?>(null);
    useEffect(() {
      inputTool.value = null;
      isEmojiPickerOpen.value = false;
      return null;
    }, [draftIdentity, draftKey]);
    final attachmentSurface = useState(_AttachmentSurface.closed);
    final iosAttachmentPopover = useMemoized(
      _IOSAttachmentPopoverController.new,
    );
    useEffect(
      () =>
          () => unawaited(iosAttachmentPopover.dispose()),
      [iosAttachmentPopover],
    );
    final isSending = useState(false);
    final showFormatting = useState(false);
    final attachments = useState<List<_PendingAttachment>>([]);
    _useOwnedAttachmentCleanup(attachments);
    final uploadError = useState<String?>(null);
    final uploadingCount = useState(0);
    final uploadProgress = useState(0.0);
    final preparingAttachments = useState(0);
    final lockedNow = useRef(interactionLocked);
    lockedNow.value = interactionLocked;
    bool rejectsAttachment() =>
        draftMembers != null &&
        (isSending.value || uploadingCount.value > 0 || lockedNow.value);
    Future<T> prepare<T>(Future<T> Function() action) =>
        _prepareComposerAttachment(context, preparingAttachments, action);
    final sending =
        isSending.value ||
        uploadingCount.value > 0 ||
        (draftMembers != null && preparingAttachments.value > 0);
    _useComposerBusy(
      isSending: isSending,
      uploadingCount: uploadingCount,
      preparingAttachments: preparingAttachments,
      isDraft: draftMembers != null,
      onChanged: onSendingChanged,
    );
    final uploadGeneration = useRef(0);
    final activeUploadCancellation = useRef<UploadCancellationToken?>(null);
    final voiceNote = _useComposerVoiceNote(
      controller: controller,
      inputTool: inputTool,
      scope: '$draftIdentity:$draftKey',
      draftKey: draftKey,
      context: context,
      ref: ref,
      focusNode: focusNode,
      isComposerExpanded: isComposerExpanded,
      showFormatting: showFormatting,
      attachmentSurface: attachmentSurface,
      uploadError: uploadError,
      draftRevision: draftRevision,
      attachments: attachments,
    );
    final voiceNoteRef = useRef(voiceNote)..value = voiceNote;
    final mentions = useListenableSelector(
      controller,
      () => controller.mentions,
    );
    final prefillText = useRef<String?>(null);
    _useComposeDraftLifecycle(
      context: context,
      voiceNote: voiceNoteRef,
      prefillText: prefillText,
      ref: ref,
      controller: controller,
      draftKey: draftKey,
      channelId: channelId,
      threadHeadId: threadHeadId,
      draftIdentity: draftIdentity,
      draftRevision: draftRevision,
      attachments: attachments,
      uploadGeneration: uploadGeneration,
      activeUploadCancellation: activeUploadCancellation,
      uploadingCount: uploadingCount,
      isSending: isSending,
      attachmentSurface: attachmentSurface,
      uploadError: uploadError,
      iosAttachmentPopover: iosAttachmentPopover,
      onDraftIdentityChanged: voiceNote.onDraftIdentityChanged,
    );
    final clipboardHasImage = useState(false);
    final hasAttachments = attachments.value.isNotEmpty;
    final customEmoji = ref.watch(customEmojiListProvider);
    final reducedMotion = MediaQuery.disableAnimationsOf(context);
    final composerExpansionController = useAnimationController(
      initialValue: 0,
      upperBound: 1.05,
    );

    void collapseComposer() {
      if (!isComposerExpanded.value || inputTool.value != null) return;
      showFormatting.value = false;
      isComposerExpanded.value = false;
    }

    useEffect(() {
      void collapseWhenUnfocused() {
        if (!focusNode.hasFocus && !isEmojiPickerOpen.value) {
          collapseComposer();
        }
      }

      focusNode.addListener(collapseWhenUnfocused);
      return () => focusNode.removeListener(collapseWhenUnfocused);
    }, [focusNode]);

    _useComposerKeyboardMetrics(
      context: context,
      focusNode: focusNode,
      voiceNote: voiceNote,
      androidImeFallbackTimer: androidImeFallbackTimer,
      androidImeTransitionStarted: androidImeTransitionStarted,
      collapseComposer: collapseComposer,
      keepKeyboardSpace: () =>
          inputTool.value != null || isEmojiPickerOpen.value,
    );
    final resolvedHint = hintText ?? 'Message';
    _useComposerExpansionMotion(
      controller: composerExpansionController,
      isExpanded: isComposerExpanded,
      androidImeTransitionStarted: androidImeTransitionStarted,
      reducedMotion: reducedMotion,
    );
    _useComposerClipboardImage(
      context: context,
      ref: ref,
      focusNode: focusNode,
      clipboardHasImage: clipboardHasImage,
    );

    // Channel autocomplete state ----------------------------------------------
    final emojiQuery = useState<String?>(null);
    final channelQuery = useState<String?>(null);
    final channelStartIdx = useState(-1);
    final channelsAsync = ref.watch(channelsProvider);
    _useComposerChannelNames(controller, channelsAsync);

    final membersAsync = draftMembers == null
        ? ref.watch(channelMembersProvider(channelId))
        : AsyncData(draftMembers!);
    final sessionStatus = ref.watch(relaySessionProvider).status;
    final cachedMembers = channelsAsync.asData == null
        ? const <ChannelMember>[]
        : ref
              .read(channelsProvider.notifier)
              .cachedMembersForChannel(channelId);
    final currentPubkey = ref.watch(currentPubkeyProvider);
    final userCache = ref.watch(userCacheProvider);
    final composeChannel = ref.watch(channelByIdProvider(channelId));
    final isDmChannel = composeChannel?.isDm ?? false;

    // Preload profiles for channel members, mentionable agents, and their
    // owners so @mention suggestions show names ("managed by …" included).
    final relayAgents = ref.watch(agentDirectoryProvider).asData?.value;
    final agentOwners = ref.watch(agentOwnersProvider).asData?.value;
    final ownedAgents = ref.watch(ownedAgentDirectoryProvider).asData?.value;
    List<MentionCandidate> memberMentionCandidates() => _typedMemberCandidates(
      membersAsync: membersAsync,
      sessionStatus: sessionStatus,
      cachedMembers: cachedMembers,
      userCache: userCache,
      agentOwners: agentOwners,
    );

    final memberMentions = _useMemberMentions(memberMentionCandidates, [
      membersAsync.asData?.value,
      sessionStatus,
      cachedMembers,
      userCache,
      agentOwners,
    ]);

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
    // Bindings migrated from a label-keyed draft give way to longer member
    // names as the roster arrives, and are ordinary once it has loaded.
    final hasLegacyMentions = mentions.any((mention) => mention.legacy);
    final membersLoaded = membersAsync.hasValue;
    useEffect(
      () {
        if (!hasLegacyMentions) return null;
        // After the frame: the change is saved, which a build must not do.
        WidgetsBinding.instance.addPostFrameCallback((_) {
          if (!context.mounted) return;
          controller.settleLegacyMentions(
            memberMentions,
            complete: membersLoaded,
          );
        });
        return null;
      },
      [
        controller,
        hasLegacyMentions,
        membersLoaded,
        membersAsync.asData?.value,
        cachedMembers,
        userCache,
      ],
    );
    useEffect(
      () {
        final memberList = channelMembersForAutocomplete(
          membersAsync: membersAsync,
          sessionStatus: sessionStatus,
          cachedMembers: cachedMembers,
        );
        final pubkeys = [
          ...memberList.map((m) => m.pubkey),
          for (final m in controller.bindings)
            if (m.candidate.requiresRevalidation) m.candidate.pubkey,
          ...?relayAgents?.map((a) => a.pubkey),
          ...?agentOwners?.values,
        ];
        if (pubkeys.isNotEmpty) {
          ref.read(userCacheProvider.notifier).preload(pubkeys);
        }
        return null;
      },
      [
        draftIdentity,
        draftKey,
        membersAsync.asData?.value.length,
        cachedMembers.length,
        relayAgents?.length,
        agentOwners?.length,
      ],
    );

    // Typing indicator broadcast — throttled to one event per 3 seconds.
    final lastTypingSentMs = useRef(0);
    final isModifyingText = useRef(false);
    final lastObservedEditingValue = useRef(controller.value);

    _usePlatformUndoBindings(controller, focusNode);
    // The `@` chooser: query, rows and chip insertion, shared with the
    // message edit sheet.
    final mentionChooser = _useMentionChooser(
      ref,
      channelId: channelId,
      controller: controller,
      focusNode: focusNode,
      isModifyingText: isModifyingText,
      viewer: currentPubkey,
    );

    _useAgentMentionPrefill(
      ref: ref,
      channelId: channelId,
      scopeMessages: prefillScopeMessages,
      draftKey: draftKey,
      controller: controller,
      prefillText: prefillText,
      isModifyingText: isModifyingText,
      hasAttachments: hasAttachments,
    );

    // Detect @mention query and broadcast typing on text / selection change.
    useEffect(() {
      lastObservedEditingValue.value = controller.value;
      void listener() {
        final editingValue = controller.value;
        final previousValue = lastObservedEditingValue.value;
        // A notify without an edit, such as new channel names, must not
        // reopen a query the user dismissed.
        if (editingValue == previousValue) return;
        lastObservedEditingValue.value = editingValue;
        emojiQuery.value = _emojiQuery(editingValue);
        if (isModifyingText.value) return;
        final text = editingValue.text;
        final sel = editingValue.selection;
        final textChanged = text != previousValue.text;

        // Broadcast typing indicator (throttled).
        if (draftMembers == null && textChanged && text.isNotEmpty) {
          final now = DateTime.now().millisecondsSinceEpoch;
          if (now - lastTypingSentMs.value > _typingThrottleMs) {
            lastTypingSentMs.value = now;
            _sendTypingIndicator(
              ref,
              channelId: channelId,
              threadHeadId: threadHeadId,
              rootId: rootId,
            );
          }
        }

        // Channel autocomplete detection — only when no @mention is active.
        if (mentionChooser.track(previousValue, editingValue) ||
            !sel.isValid ||
            !sel.isCollapsed ||
            sel.baseOffset < 1) {
          channelQuery.value = null;
          return;
        }
        final cursor = sel.baseOffset;
        final hashPos = findTrigger(text, cursor, '#');
        if (hashPos != null) {
          channelQuery.value = text
              .substring(hashPos + 1, cursor)
              .toLowerCase();
          channelStartIdx.value = hashPos;
        } else {
          channelQuery.value = null;
        }
      }

      controller.addListener(listener);
      return () => controller.removeListener(listener);
    }, [controller]);

    final mentionRows = mentionChooser.rows;
    final suggestions = mentionRows.rows;

    // Filter channels against the query.
    final channels = channelsAsync.asData?.value ?? <Channel>[];
    final channelSuggestions = filterChannels(channels, channelQuery.value);

    // Load the catalog on the first colon query, then keep it so the next
    // token reuses the index instead of rebuilding it.
    final emojiIndexWanted = useRef(false);
    if (emojiQuery.value != null) emojiIndexWanted.value = true;
    final dataset = emojiIndexWanted.value
        ? ref.watch(emojiDatasetOrEmptyProvider)
        : null;
    final emojiIndex = useMemoized(
      () => dataset == null
          ? null
          : ShortcodeIndex<_EmojiChoice>([
              for (final entry in customEmoji)
                (
                  code: entry.shortcode,
                  text: ':${entry.shortcode}:',
                  url: entry.url,
                ),
              for (final entry in dataset.all)
                if (entry.skinIndex == 0)
                  (code: entry.id, text: entry.native, url: null),
            ], (entry) => entry.code),
      [dataset, customEmoji],
    );
    final emojiSuggestions = useMemoized(() {
      final query = emojiQuery.value;
      if (query == null || emojiIndex == null) return const <_EmojiChoice>[];
      return emojiIndex.rank(query, limit: _emojiSuggestionLimit);
    }, [emojiQuery.value, emojiIndex]);

    void selectEmoji(_EmojiChoice choice) {
      final query = _emojiQuery(controller.value);
      if (query == null || query != emojiQuery.value) return;
      final caret = controller.selection.end;
      final start = caret - query.length - 1;
      final suffix = _emojiSuffix.matchAsPrefix(controller.text, caret)!;
      _replaceComposerRange(
        controller,
        start: start,
        end: suffix.end,
        replacement: choice.text,
        caret: start + choice.text.length,
      );
      focusNode.requestFocus();
      emojiQuery.value = null;
    }

    // Insert a selected channel into the text field.
    void insertChannel(Channel channel) {
      final start = channelStartIdx.value.clamp(0, controller.text.length);
      isModifyingText.value = true;
      try {
        spliceAndMoveCursor(
          controller,
          focusNode,
          start: start,
          replacement: '#${channel.name} ',
        );
      } finally {
        isModifyingText.value = false;
      }
      channelQuery.value = null;
      inputTool.value = null;
      unawaited(SystemChannels.textInput.invokeMethod<void>('TextInput.show'));
    }

    // Insert `@` at the cursor to manually trigger mention mode.
    void triggerMention() => _insertTriggerAtCursor(controller, focusNode, '@');

    // Insert `#` at the cursor to manually trigger channel mode.
    void triggerChannel() => _insertTriggerAtCursor(controller, focusNode, '#');

    // A picked row returns the composer to the keyboard, as a channel does.
    void insertMention(MentionCandidate candidate) {
      mentionChooser.insert(candidate);
      inputTool.value = null;
      unawaited(SystemChannels.textInput.invokeMethod<void>('TextInput.show'));
    }

    void insertEmoji(String emoji) => _insertComposerEmoji(
      controller,
      focusNode,
      emoji,
      restoreFocus: !isEmojiPickerOpen.value,
    );

    void clearComposer() {
      draftRevision.value += 1;
      controller.replaceDraft(TextEditingValue.empty, mentions: const []);
      attachments.value = [];
      mentionChooser.close();
      channelQuery.value = null;
      attachmentSurface.value = _AttachmentSurface.closed;
      showFormatting.value = false;
      uploadError.value = null;
      focusNode.requestFocus();
    }

    void removeAttachment(int id) {
      _removePendingAttachment(attachments, draftRevision, id);
    }

    bool canSend() =>
        !(draftMembers != null && preparingAttachments.value > 0) &&
        !interactionLocked &&
        (controller.text.trim().isNotEmpty || hasAttachments) &&
        !isSending.value &&
        uploadingCount.value == 0;

    // Send the message.
    Future<void> send() async {
      if (!canSend()) return;
      // A draft saved by an earlier Buzz yields to member names first.
      controller.settleLegacyMentions(memberMentions, complete: false);
      final draft = controller.text;
      final mentions = controller.mentions;
      final code = controller._styledCodeRanges(draft);
      final submittedOrigin = ComposerSendOrigin.capture(sendSurfaceKey);
      Future<void> sendWithOrigin(
        String content,
        List<String> mentionPubkeys, {
        List<List<String>> mediaTags = const [],
      }) => ComposerSendOrigin.run(
        submittedOrigin,
        () => onSend(content, mentionPubkeys, mediaTags: mediaTags),
      );
      final submittedDraftRevision = draftRevision.value;
      final cleanupStoredDraft = ref
          .read(composeDraftsProvider.notifier)
          .cleanupAfterSend(draftKey);
      // Resolved before any await: see
      // `_reportSendCancelledByCommunitySwitch`.
      final messenger = ScaffoldMessenger.maybeOf(context);

      // The recipients: every chip, then typed names the fallback resolves.
      final List<MentionCandidate> selectedMentions;
      final Map<int, int> nameEnds;
      try {
        (recipients: selectedMentions, :nameEnds) = _resolveComposerMentions(
          draft,
          mentions,
          mentionsInvalid: controller.mentionsInvalid,
          members: memberMentions(),
          code: code,
          restoredCandidates: buildMentionCandidates(
            members: membersAsync.asData?.value ?? const [],
            relayAgents: relayAgents ?? const [],
            sharedChannelIds: {
              for (final c in channels)
                if (c.isMember && !c.isArchived) c.id,
            },
            userCache: userCache,
            ownerByAgentPubkey: agentOwners ?? const {},
            currentPubkey: currentPubkey,
            ownedAgents: ownedAgents,
            offerOwnedAgents:
                composeChannel != null &&
                (composeChannel.isStream || composeChannel.isForum),
            // Reuse ordinary search-result classification, not membership as
            // permission. Persisted keys/flags themselves prove no role.
            searchResults: [
              for (final m in mentions)
                if (m.candidate.requiresRevalidation)
                  ?userCache[m.candidate.pubkey],
            ],
          ),
        );
      } on FormatException catch (error) {
        messenger?.showFeedbackSnackBar(
          SnackBar(content: Text(error.message)),
          kind: ToastKind.error,
        );
        return;
      }
      // Namesakes of different keys are told apart on the wire; the
      // composer itself never showed the key.
      final text = _qualifiedMentionContent(
        draft,
        mentions,
        nameEnds: nameEnds,
      ).trim();
      final outgoing = _OutgoingMentions(selectedMentions);
      // Read before any await. These actions belong to this community and
      // refuse to run after a switch, so an Invite answered after a switch
      // cannot add people in the new community.
      final channelActions = ref.read(channelActionsProvider);
      final _NonMemberMentionScan scan;
      try {
        scan = draftMembers != null
            ? _NonMemberMentionScan(
                channelId: channelId,
                outside: selectedMentions
                    .where(
                      (candidate) => !draftMembers!.any(
                        (member) => member.pubkey == candidate.pubkey,
                      ),
                    )
                    .toList(),
                canAddMembers: false,
                isDm: true,
              )
            : await _scanNonMemberMentions(
                ref,
                channelId: channelId,
                selectedMentions: selectedMentions,
                currentPubkey: currentPubkey,
              );
      } catch (_) {
        // Nothing was sent and the draft is untouched, so Retry sends it again.
        if (context.mounted) {
          showBuzzToast(
            context,
            'Not sent. Couldn’t check members.',
            kind: ToastKind.error,
            onRetry: () => unawaited(send()),
          );
        }
        return;
      }

      // Mentioning anyone outside the channel, person or agent, prompts
      // "Invite" / "Do nothing" (portable mention rules, section 7). Nobody
      // can be added to a DM, so a DM sends them as references without asking.
      if (scan.outside.isNotEmpty && scan.isDm) {
        outgoing.resolveOutsideChoice(
          _NonMemberMentionChoice.sendWithoutInviting,
          scan.outside,
        );
      } else if (scan.outside.isNotEmpty) {
        if (!context.mounted) return;
        final choice = await _promptNonMemberMention(
          context,
          names: [for (final candidate in scan.outside) candidate.label],
          canInvite: scan.canAddMembers,
        );
        if (choice == null) return; // Dismissed — keep the draft, send nothing.
        outgoing.resolveOutsideChoice(choice, scan.outside);
      }

      final queuedAttachments = List<_PendingAttachment>.of(attachments.value);

      // An add that was refused doesn't block the message: it is reported and
      // the un-added mentions are demoted to reference tags so the send lands.
      Future<void> addMentionedNonMembers() => outgoing.addNonMembers(
        channelActions,
        scan: scan,
        messenger: messenger,
      );

      // Another submission may have finished the same asynchronous checks.
      if (!context.mounted || isSending.value) return;
      isSending.value = true;
      try {
        if (queuedAttachments.isEmpty) {
          if (!context.mounted) return;
          await _sendTextOnlyDraft(
            context: context,
            controller: controller,
            draftRevision: draftRevision,
            submittedDraftRevision: submittedDraftRevision,
            cleanupStoredDraft: cleanupStoredDraft,
            focusNode: focusNode,
            clearComposer: clearComposer,
            addMentionedNonMembers: addMentionedNonMembers,
            payload: _ComposeDraftPayload.fromDraft(
              text: text,
              attachments: const [],
              customEmoji: customEmoji,
            ),
            outgoing: outgoing,
            onSend: sendWithOrigin,
            messenger: messenger,
          );
          return;
        }

        final draftText = controller.value;
        final draftAttachments = List<_PendingAttachment>.of(attachments.value);
        final draftMentions = controller.bindings;
        final draftMentionsInvalid = controller.mentionsInvalid;
        clearComposer();
        final clearedDraftRevision = draftRevision.value;
        uploadingCount.value += 1;
        uploadProgress.value = 0;
        isSending.value = false;
        final queueGeneration = uploadGeneration.value;
        final cancellation = UploadCancellationToken();
        final uploadService = ref.read(mediaUploadServiceProvider);
        activeUploadCancellation.value = cancellation;
        final delivery = sendWithOrigin;
        unawaited(() async {
          var retainedForRetry = false;
          try {
            final uploaded = <BlobDescriptor>[];
            for (var index = 0; index < queuedAttachments.length; index++) {
              final attachment = queuedAttachments[index];
              final descriptor = await _uploadPendingAttachment(
                uploadService,
                attachment,
                onProgress: (progress) {
                  if (context.mounted) {
                    uploadProgress.value =
                        (index + progress) / queuedAttachments.length;
                  }
                },
                cancellationToken: cancellation,
              );
              if (queueGeneration != uploadGeneration.value) return;
              uploaded.add(descriptor);
              if (context.mounted) {
                uploadProgress.value = (index + 1) / queuedAttachments.length;
              }
            }
            final payload = _ComposeDraftPayload.fromDraft(
              text: text,
              attachments: uploaded,
              customEmoji: customEmoji,
            );
            if (queueGeneration != uploadGeneration.value) return;
            await addMentionedNonMembers();
            if (queueGeneration != uploadGeneration.value) return;
            await delivery(
              payload.content,
              outgoing.pubkeys,
              mediaTags: [...payload.mediaTags, ...outgoing.referenceTags],
            );
          } catch (error) {
            if (cancellation.isCancelled) return;
            if (context.mounted) uploadError.value = _formatUploadError(error);
            if (context.mounted &&
                queueGeneration == uploadGeneration.value &&
                draftRevision.value == clearedDraftRevision) {
              attachments.value = draftAttachments;
              retainedForRetry = true;
              controller.replaceDraft(
                draftText,
                mentions: draftMentions,
                mentionsInvalid: draftMentionsInvalid,
              );
              focusNode.requestFocus();
            }
          } finally {
            if (!retainedForRetry) {
              await _deleteOwnedAttachments(queuedAttachments);
            }
            if (activeUploadCancellation.value == cancellation) {
              activeUploadCancellation.value = null;
            }
            if (context.mounted && queueGeneration == uploadGeneration.value) {
              uploadingCount.value = math.max(0, uploadingCount.value - 1);
            }
          }
        }());
      } finally {
        if (context.mounted && isSending.value) isSending.value = false;
      }
    }

    final queueAttachment = useCallback((
      XFile file,
      _PendingAttachmentKind kind, {
      bool deleteAfterUse = false,
    }) {
      if (rejectsAttachment()) {
        if (deleteAfterUse) unawaited(_deleteXFile(file));
        return;
      }
      _queueComposerAttachment(
        file,
        kind,
        voiceNoteRef,
        attachments,
        uploadError,
        draftRevision,
        deleteAfterUse: deleteAfterUse,
      );
    }, [voiceNoteRef, draftRevision, uploadError, attachments]);
    Future<void> pickThenQueue({
      required Future<XFile?> Function() pick,
      required _PendingAttachmentKind kind,
    }) async {
      uploadError.value = null;
      try {
        final picked = await pick();
        if (picked == null || !context.mounted) return;
        queueAttachment(picked, kind);
      } catch (error) {
        if (context.mounted) {
          uploadError.value = _formatUploadError(error);
        }
      }
    }

    bool queueImages(List<XFile> images, {bool deleteAfterUse = false}) =>
        !rejectsAttachment() &&
        _queueComposerImages(
          images,
          voiceNote,
          attachments,
          uploadError,
          draftRevision,
          deleteAfterUse,
        );

    Future<void> retainAndQueueImages(List<XFile> images) =>
        prepare(() => _retainAndQueueImages(context, images, queueImages));

    final pasteClipboardImage = useCallback(() {
      ContextMenuController.removeAny();
      unawaited(
        prepare(() async {
          try {
            final image = await ref
                .read(mediaUploadServiceProvider)
                .readClipboardImage();
            if (image != null && context.mounted) {
              queueAttachment(image, _PendingAttachmentKind.image);
            } else if (context.mounted) {
              uploadError.value = 'Unable to read pasted image';
            }
          } catch (error) {
            if (context.mounted) uploadError.value = _formatUploadError(error);
          }
        }),
      );
    }, [context, ref, queueAttachment, uploadError]);
    void format(String prefix, [String? suffix]) => _applyComposerFormat(
      controller,
      focusNode,
      isModifyingText,
      prefix,
      suffix,
    );
    final buildContextMenu = useCallback<EditableTextContextMenuBuilder>((
      context,
      editable,
    ) {
      return _ComposerSelectionMenu(
        editable: editable,
        onFormat: format,
        clipboardHasImage: clipboardHasImage.value,
        onPasteImage: pasteClipboardImage,
      );
    }, [clipboardHasImage, pasteClipboardImage, controller, focusNode]);
    void uploadPastedImage(KeyboardInsertedContent content) {
      final bytes = content.data;
      if (bytes == null || bytes.isEmpty) {
        uploadError.value = 'Unable to read pasted image';
        return;
      }

      queueAttachment(
        XFile.fromData(bytes, name: 'Pasted image'),
        _PendingAttachmentKind.image,
      );
    }

    void chooseAttachment(
      Future<void> Function() choose, {
      String? errorMessage,
    }) => _rejectsNonVoiceAttachment(voiceNote, attachments.value, uploadError)
        ? attachmentSurface.value = _AttachmentSurface.closed
        : _chooseComposerAttachment(
            context,
            attachmentSurface,
            uploadError,
            () => prepare(choose),
            inputTool: inputTool,
            errorMessage: errorMessage,
          );
    void toggleAttachments() =>
        _toggleComposerAttachments(attachmentSurface, inputTool);

    void openCamera() {
      if (defaultTargetPlatform == TargetPlatform.iOS) {
        inputTool.value = _InputTool.attachments;
      }
      focusNode.unfocus();
      attachmentSurface.value = _AttachmentSurface.camera;
    }

    final motionDuration = _composerMotionDuration(
      reducedMotion,
      attachmentSurface.value,
    );
    final resizeDuration = reducedMotion
        ? Duration.zero
        : const Duration(milliseconds: 140);
    final suggestionOverlayController = _useComposerSuggestionPortal(context);
    void expandComposer() => _expandComposer(
      context: context,
      isExpanded: isComposerExpanded,
      attachmentSurface: attachmentSurface,
      onFocusRequested: onFocusRequested,
      focusNode: focusNode,
      view: View.of(context),
      androidImeTransitionStarted: androidImeTransitionStarted,
      androidImeFallbackTimer: androidImeFallbackTimer,
    );

    _useComposerFocusRestorer(
      onChanged: onFocusRestorerChanged,
      isExpanded: isComposerExpanded,
      focusNode: focusNode,
      expand: expandComposer,
    );

    final readOnly = interactionLocked || (draftMembers != null && sending);
    // The chooser belongs to the focused field (recommendation 8). It hides
    // while the field has no focus, after the keyboard is dismissed or a tap
    // lands elsewhere, and comes back unchanged when focus returns: the query,
    // its rows and its search are kept, only the panel is withheld. A tap on
    // a row is a touch outside the field, which never takes its focus.
    final chooserOpen = useListenable(focusNode).hasFocus;
    // Matches what _composerSuggestionPanel renders, and only while the field
    // can be edited.
    final pickerVisible =
        chooserOpen &&
        (emojiSuggestions.isNotEmpty ||
            channelSuggestions.isNotEmpty ||
            suggestions.isNotEmpty ||
            mentionRows.searchFailed) &&
        attachmentSurface.value == _AttachmentSurface.closed &&
        !readOnly;
    final topChannel = channelSuggestions.firstOrNull;
    final topMention = suggestions
        .where((s) => !mentionRows.unavailable.contains(s.pubkey))
        .firstOrNull;
    String? activeQuery() =>
        emojiQuery.value == null ? channelQuery.value : ':${emojiQuery.value}';
    final pickerQuery = (mentionChooser.query, activeQuery());
    KeyEventResult handleHardwareKey(FocusNode _, KeyEvent event) =>
        _handleComposerHardwareKey(
          event,
          controller,
          inputOwned: iosAttachmentPopover.isActive || isEmojiPickerOpen.value,
          query: (mentionChooser.liveQuery.value, activeQuery()),
          renderedQuery: pickerQuery,
          pickerVisible: pickerVisible,
          acceptSuggestion: emojiSuggestions.isNotEmpty
              ? () => _runComposerAction(
                  () => selectEmoji(emojiSuggestions.first),
                )
              : topChannel != null
              ? () => _runComposerAction(() => insertChannel(topChannel))
              : topMention != null
              ? () => _runComposerAction(() => insertMention(topMention))
              : null,
          send: () async {
            if (attachmentSurface.value != _AttachmentSurface.closed) return;
            if (canSend()) _runComposerAction(() => unawaited(send()));
          },
        );

    final suggestionPanel = chooserOpen && emojiSuggestions.isNotEmpty
        ? TextFieldTapRegion(
            child: _ComposerSearchSuggestions<_EmojiChoice>(
              surfaceKey: const ValueKey('emoji-suggestions-popover'),
              suggestions: emojiSuggestions,
              label: (entry) => ':${entry.code}:',
              horizontalTitleGap: 8,
              singleLineLabel: true,
              labelStyle: context.textTheme.bodyLarge?.copyWith(fontSize: 14),
              leading: _emojiSuggestionPreview,
              onSelect: selectEmoji,
            ),
          )
        : _composerSuggestionPanel(
            channelSuggestions: chooserOpen ? channelSuggestions : const [],
            mentionSuggestions: chooserOpen ? suggestions : const [],
            unavailableMentions: mentionRows.unavailable,
            mentionSearchFailed: chooserOpen && mentionRows.searchFailed,
            reserveMentionHeight: mentionRows.reserveHeight,
            mentionSearching: mentionRows.searching,
            onMentionSearchRetry: () {
              if (mentionChooser.query case final query?) {
                ref.invalidate(mentionUserSearchProvider(query));
              }
            },
            userCache: userCache,
            currentPubkey: currentPubkey,
            isDmChannel: isDmChannel,
            onChannelSelect: insertChannel,
            onMentionSelect: insertMention,
          );
    _AttachmentSurfacePanel buildOverlayPanel(_AttachmentSurface surface) {
      return _composerAttachmentPanel(
        preparePhotos: prepare,
        surface: surface,
        suggestionPanel: suggestionPanel,
        onBack: () => attachmentSurface.value = _AttachmentSurface.menu,
        onCamera: openCamera,
        onPhotos: () {
          focusNode.unfocus();
          attachmentSurface.value = _AttachmentSurface.photos;
        },
        onVideo: () => chooseAttachment(() {
          final service = ref.read(mediaUploadServiceProvider);
          return pickThenQueue(
            pick: service.pickGalleryVideo,
            kind: _PendingAttachmentKind.video,
          );
        }),
        onVoiceNote: () {
          inputTool.value = null;
          voiceNote.start();
        },
        onFiles: () => chooseAttachment(() {
          final service = ref.read(mediaUploadServiceProvider);
          return pickThenQueue(
            pick: service.pickAttachmentFile,
            kind: _PendingAttachmentKind.file,
          );
        }),
        onCapture: (image) async {
          inputTool.value = null;
          attachmentSurface.value = _AttachmentSurface.closed;
          await retainAndQueueImages([image]);
        },
        onPickAllPhotos: () =>
            prepare(ref.read(mediaUploadServiceProvider).pickGalleryImages),
        onChoosePhotos: (photos) async {
          inputTool.value = null;
          attachmentSurface.value = _AttachmentSurface.closed;
          await retainAndQueueImages(photos);
        },
        onChooseAllPhotos: (photos) async {
          inputTool.value = null;
          attachmentSurface.value = _AttachmentSurface.closed;
          queueImages(photos);
        },
      );
    }

    void handleAttachmentTap(BuildContext triggerContext) {
      if (defaultTargetPlatform != TargetPlatform.iOS ||
          attachmentSurface.value != _AttachmentSurface.closed) {
        toggleAttachments();
        return;
      }
      mentionChooser.close();
      channelQuery.value = null;
      final panel = buildOverlayPanel(_AttachmentSurface.closed);
      unawaited(
        iosAttachmentPopover
            .present(
              sourceContext: triggerContext,
              onCapture: (image) => retainAndQueueImages([image]),
              onChoosePhotos: retainAndQueueImages,
              onAllPhotos: () => chooseAttachment(() async {
                queueImages(
                  await ref
                      .read(mediaUploadServiceProvider)
                      .pickGalleryImages(),
                );
              }, errorMessage: 'Couldn’t open photo library'),
              onVideo: panel.onVideo,
              onVoiceNote: panel.onVoiceNote,
              onFiles: panel.onFiles,
            )
            .then((didPresent) {
              if (!didPresent && context.mounted) {
                focusNode.unfocus();
                toggleAttachments();
              }
            }),
      );
    }

    final openEmoji = _composerEmojiAction(
      context,
      focusNode,
      inputTool,
      attachmentSurface,
      isEmojiPickerOpen,
      insertEmoji,
    );

    return _ComposerDockFrame(
      expansionAnimation: composerExpansionController,
      hasDraft: draftText.isNotEmpty,
      forceFullWidth: _voiceNoteFullWidth(voiceNote, attachments.value),
      child: _ComposerInputTray(
        tool: inputTool,
        attachments: buildOverlayPanel,
        attachmentSurface: attachmentSurface,
        focusNode: focusNode,
        enabled:
            defaultTargetPlatform == TargetPlatform.iOS &&
            !interactionLocked &&
            !voiceNote.isRecording,
        suggestions:
            (chooserOpen && emojiSuggestions.isNotEmpty) ||
                mentionChooser.query != null ||
                channelQuery.value != null
            ? suggestionPanel
            : null,
        onDismissSuggestions: () {
          emojiQuery.value = null;
          mentionChooser.close();
          channelQuery.value = null;
        },
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            _UploadProgressMotion(
              visible: uploadingCount.value > 0,
              progress: uploadProgress.value,
              reducedMotion: reducedMotion,
              onCancel: () {
                activeUploadCancellation.value?.cancel();
                uploadGeneration.value += 1;
                uploadingCount.value = 0;
                uploadProgress.value = 0;
              },
            ),
            _ComposerOverlayPortal(
              controller: suggestionOverlayController,
              suppressed:
                  defaultTargetPlatform == TargetPlatform.iOS ||
                  (inputTool.value != null &&
                      inputTool.value != _InputTool.keyboard),
              attachmentSurface: attachmentSurface,
              reducedMotion: reducedMotion,
              buildOverlayPanel: buildOverlayPanel,
              onDismissAttachmentSurface: () {
                attachmentSurface.value = _AttachmentSurface.closed;
              },
              child: AbsorbPointer(
                absorbing:
                    interactionLocked || (draftMembers != null && sending),
                child: _ComposeBarLayout(
                  surfaceKey: sendSurfaceKey,
                  voiceNoteRecorder: voiceNote.recorder,
                  keyboardToolbar: defaultTargetPlatform == TargetPlatform.iOS,
                  onTextTap: () {
                    inputTool.value = null;
                    attachmentSurface.value = _AttachmentSurface.closed;
                  },
                  attachments: attachments.value,
                  onRemoveAttachment: removeAttachment,
                  uploadError: uploadError.value,
                  isExpanded: isComposerExpanded.value,
                  controller: controller,
                  focusNode: focusNode,
                  contextMenuBuilder: buildContextMenu,
                  onContentInserted: uploadPastedImage,
                  onSend: () => unawaited(send()),
                  onKeyEvent: handleHardwareKey,
                  resolvedHint: resolvedHint,
                  attachmentSurface: attachmentSurface.value,
                  onAttachmentTap: handleAttachmentTap,
                  onExpand: expandComposer,
                  expansionAnimation: composerExpansionController,
                  formattingOpen: showFormatting.value,
                  onCloseFormatting: () => showFormatting.value = false,
                  motionDuration: motionDuration,
                  resizeDuration: resizeDuration,
                  onFormat: format,
                  onMention: () {
                    attachmentSurface.value = _AttachmentSurface.closed;
                    triggerMention();
                  },
                  onChannel: () {
                    attachmentSurface.value = _AttachmentSurface.closed;
                    triggerChannel();
                  },
                  onVoiceNote: voiceNote.startTranscription,
                  onEmoji: openEmoji,
                  onOpenFormatting: () {
                    attachmentSurface.value = _AttachmentSurface.closed;
                    showFormatting.value = true;
                  },
                  hasPendingUploads: uploadingCount.value > 0,
                  canSend: draftText.trim().isNotEmpty || hasAttachments,
                  isSending:
                      isSending.value || (draftMembers != null && sending),
                  readOnly: readOnly,
                ),
              ),
            ),
          ],
        ),
      ),
    );
  }
}
