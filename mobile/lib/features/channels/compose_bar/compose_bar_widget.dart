part of '../compose_bar.dart';

class ComposeBar extends HookConsumerWidget {
  final String channelId;
  final String channelName;
  final String? hintText;
  final ComposeBarOnSend onSend;

  /// Lets a parent prepare its layout before the editor requests focus.
  final VoidCallback? onFocusRequested;

  /// Parent-owned if set; otherwise internally created and disposed.
  final FocusNode? focusNode;

  /// Receives a restorer which becomes a no-op after replacement/unmount.
  final ValueChanged<VoidCallback>? onFocusRestorerChanged;

  /// Optional thread IDs for thread-scoped typing indicators.
  final String? threadHeadId;
  final String? rootId;
  const ComposeBar({
    super.key,
    required this.channelId,
    this.channelName = '',
    this.hintText,
    this.threadHeadId,
    this.rootId,
    this.focusNode,
    this.onFocusRestorerChanged,
    this.onFocusRequested,
    required this.onSend,
  });
  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final controller = useMemoized(_MarkdownEditingController.new);
    final composerText = useListenableSelector(
      controller,
      () => controller.text,
    );
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
    useEffect(
      () => () {
        androidImeFallbackTimer.value?.cancel();
        _dismissComposerKeyboard(focusNode);
      },
      [focusNode],
    );
    final isEmojiPickerOpen = useState(false);
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
    final uploadGeneration = useRef(0);
    final activeUploadCancellation = useRef<UploadCancellationToken?>(null);
    final voiceNote = _useComposerVoiceNote(
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
    _useComposeDraftLifecycle(
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
      if (!isComposerExpanded.value) return;
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

    final appView = View.of(context);
    useEffect(() {
      final observer = _ComposerKeyboardMetricsObserver(
        view: appView,
        onKeyboardShown: () {
          androidImeFallbackTimer.value?.cancel();
          androidImeTransitionStarted.value = true;
        },
        onKeyboardHidden: () {
          androidImeFallbackTimer.value?.cancel();
          if (defaultTargetPlatform == TargetPlatform.android) {
            androidImeTransitionStarted.value = false;
          }
          voiceNote.onKeyboardHidden();
          collapseComposer();
          focusNode.unfocus();
        },
      );
      WidgetsBinding.instance.addObserver(observer);
      return () => WidgetsBinding.instance.removeObserver(observer);
    }, [appView, focusNode, voiceNote.isPreparing]);
    final resolvedHint =
        hintText ??
        (channelName.isNotEmpty ? 'Message #$channelName' : 'Message\u2026');
    useEffect(
      () {
        final target =
            isComposerExpanded.value && androidImeTransitionStarted.value
            ? 1.0
            : 0.0;
        if (reducedMotion) {
          composerExpansionController.value = target;
        } else if ((composerExpansionController.value - target).abs() > 0.001) {
          composerExpansionController.animateWith(
            SpringSimulation(
              SpringDescription.withDurationAndBounce(
                duration: const Duration(milliseconds: 220),
                bounce: 0.08,
              ),
              composerExpansionController.value,
              target,
              0,
              snapToEnd: true,
            ),
          );
        }
        return null;
      },
      [
        isComposerExpanded.value,
        androidImeTransitionStarted.value,
        reducedMotion,
      ],
    );
    useEffect(() {
      if (defaultTargetPlatform != TargetPlatform.iOS) return null;

      var disposed = false;
      Future<void> refreshClipboardAvailability() async {
        final hasImage = await ref
            .read(mediaUploadServiceProvider)
            .clipboardHasImage();
        if (!disposed && context.mounted) {
          clipboardHasImage.value = hasImage;
        }
      }

      void refreshWhenFocused() {
        if (focusNode.hasFocus) refreshClipboardAvailability();
      }

      final lifecycleListener = AppLifecycleListener(
        onResume: refreshClipboardAvailability,
      );
      focusNode.addListener(refreshWhenFocused);
      refreshClipboardAvailability();
      return () {
        disposed = true;
        focusNode.removeListener(refreshWhenFocused);
        lifecycleListener.dispose();
      };
    }, [focusNode]);

    // Mention state --------------------------------------------------------
    final mentionQuery = useState<String?>(null);
    final mentionStartIdx = useState(-1);
    // Map of displayName → selected mention candidate built as the user selects
    // mentions. Used to pass resolved pubkeys directly to onSend and to attach
    // selected non-member agents before the message is published.
    final mentionMap = useRef(<String, MentionCandidate>{});

    // Channel autocomplete state ----------------------------------------------
    final channelQuery = useState<String?>(null);
    final channelStartIdx = useState(-1);
    final channelsAsync = ref.watch(channelsProvider);
    _useComposerChannelNames(controller, channelsAsync);

    final membersAsync = ref.watch(channelMembersProvider(channelId));
    final sessionStatus = ref.watch(relaySessionProvider).status;
    final cachedMembers = channelsAsync.asData == null
        ? const <ChannelMember>[]
        : ref
              .read(channelsProvider.notifier)
              .cachedMembersForChannel(channelId);
    final currentPubkey = ref.watch(currentPubkeyProvider);
    final userCache = ref.watch(userCacheProvider);
    final isDmChannel =
        channelsAsync.asData?.value.any((c) => c.id == channelId && c.isDm) ??
        false;

    // Preload profiles for channel members, mentionable agents, and their
    // owners so @mention suggestions show names ("managed by …" included).
    final relayAgents = ref.watch(agentDirectoryProvider).asData?.value;
    final agentOwners = ref.watch(agentOwnersProvider).asData?.value;
    final agentMentionLabels = _agentMentionLabels(
      candidates: mentionMap.value.values,
    );
    final agentMentionLabelsKey = (agentMentionLabels.toList()..sort()).join(
      '\u0000',
    );
    useEffect(() {
      controller.setAgentMentionNames(agentMentionLabels);
      return null;
    }, [controller, agentMentionLabelsKey]);
    useEffect(
      () {
        final memberList = channelMembersForAutocomplete(
          membersAsync: membersAsync,
          sessionStatus: sessionStatus,
          cachedMembers: cachedMembers,
        );
        final pubkeys = [
          ...memberList.map((m) => m.pubkey),
          ...?relayAgents?.map((a) => a.pubkey),
          ...?agentOwners?.values,
        ];
        if (pubkeys.isNotEmpty) {
          ref.read(userCacheProvider.notifier).preload(pubkeys);
        }
        return null;
      },
      [
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

    // Detect @mention query and broadcast typing on text / selection change.
    useEffect(() {
      lastObservedEditingValue.value = controller.value;
      void listener() {
        final editingValue = controller.value;
        final previousValue = lastObservedEditingValue.value;
        lastObservedEditingValue.value = editingValue;
        if (isModifyingText.value || editingValue == previousValue) return;
        final text = editingValue.text;
        final sel = editingValue.selection;
        final textChanged = text != previousValue.text;

        // Broadcast typing indicator (throttled).
        if (textChanged && text.isNotEmpty) {
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

        if (!sel.isValid || !sel.isCollapsed) {
          mentionQuery.value = null;
          channelQuery.value = null;
          return;
        }
        final cursor = sel.baseOffset;
        if (cursor < 1) {
          mentionQuery.value = null;
          channelQuery.value = null;
          return;
        }

        // Walk backward from cursor looking for trigger characters.
        // stopAtSpace: false — @mentions support multi-word display names.
        final atPos = findTrigger(text, cursor, '@', stopAtSpace: false);

        if (atPos != null) {
          mentionQuery.value = text.substring(atPos + 1, cursor).toLowerCase();
          mentionStartIdx.value = atPos;
          channelQuery.value = null;
        } else {
          mentionQuery.value = null;
        }

        // Channel autocomplete detection — only when no @mention is active.
        if (mentionQuery.value == null) {
          final hashPos = findTrigger(text, cursor, '#');
          if (hashPos != null) {
            channelQuery.value = text
                .substring(hashPos + 1, cursor)
                .toLowerCase();
            channelStartIdx.value = hashPos;
          } else {
            channelQuery.value = null;
          }
        } else {
          channelQuery.value = null;
        }
      }

      controller.addListener(listener);
      return () => controller.removeListener(listener);
    }, [controller]);

    // Ranked mention candidates (desktop-parity ordering + eligibility).
    final suggestions = mentionQuery.value == null
        ? const <MentionCandidate>[]
        : ref
              .watch(
                mentionCandidatesProvider((
                  channelId: channelId,
                  query: mentionQuery.value!,
                )),
              )
              .take(_mentionSuggestionLimit)
              .toList();

    // Resolve owner names for the visible "managed by …" subtitles.
    useEffect(() {
      final ownerPubkeys = [for (final s in suggestions) ?s.ownerPubkey];
      if (ownerPubkeys.isNotEmpty) {
        ref.read(userCacheProvider.notifier).preload(ownerPubkeys);
      }
      return null;
    }, [suggestions.length, mentionQuery.value]);

    // Filter channels against the query.
    final channels = channelsAsync.asData?.value ?? <Channel>[];
    final channelSuggestions = filterChannels(channels, channelQuery.value);

    // Insert a selected mention into the text field.
    void insertMention(MentionCandidate candidate) {
      final name = candidate.label;
      // Track the resolved candidate so we can pass its pubkey and prepare
      // selected non-member agents at send time.
      mentionMap.value[name] = candidate;

      final start = mentionStartIdx.value.clamp(0, controller.text.length);
      isModifyingText.value = true;
      try {
        spliceAndMoveCursor(
          controller,
          focusNode,
          start: start,
          replacement: '@$name ',
        );
      } finally {
        isModifyingText.value = false;
      }
      mentionQuery.value = null;
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
    }

    // Insert `@` at the cursor to manually trigger mention mode.
    void triggerMention() => _insertTriggerAtCursor(controller, focusNode, '@');

    // Insert `#` at the cursor to manually trigger channel mode.
    void triggerChannel() => _insertTriggerAtCursor(controller, focusNode, '#');

    // Insert a selected emoji at the cursor without replacing the draft.
    void insertEmoji(String emoji) {
      final text = controller.text;
      final selection = controller.selection;
      final cursor = selection.isValid
          ? selection.baseOffset.clamp(0, text.length)
          : text.length;
      controller.value = TextEditingValue(
        text: text.replaceRange(cursor, cursor, emoji),
        selection: TextSelection.collapsed(offset: cursor + emoji.length),
      );
      focusNode.requestFocus();
    }

    void clearComposer() {
      draftRevision.value += 1;
      controller.clear();
      attachments.value = [];
      mentionMap.value.clear();
      mentionQuery.value = null;
      channelQuery.value = null;
      attachmentSurface.value = _AttachmentSurface.closed;
      showFormatting.value = false;
      uploadError.value = null;
      focusNode.requestFocus();
    }

    void removeAttachment(int id) {
      _removePendingAttachment(attachments, draftRevision, id);
    }

    // Send the message.
    Future<void> send() async {
      final text = controller.text.trim();
      if ((text.isEmpty && !hasAttachments) ||
          isSending.value ||
          uploadingCount.value > 0) {
        return;
      }
      final submittedDraftRevision = draftRevision.value;
      // Resolved before any await: see
      // `_reportSendCancelledByCommunitySwitch`.
      final messenger = ScaffoldMessenger.maybeOf(context);

      // Extract pubkeys for mentions present in the final text.
      final selectedMentions = <MentionCandidate>[
        for (final entry in mentionMap.value.entries)
          if (hasMention(text, entry.key)) entry.value,
      ];
      final outgoing = _OutgoingMentions(selectedMentions);
      final scan = await _scanNonMemberMentions(
        ref,
        channelId: channelId,
        selectedMentions: selectedMentions,
        currentPubkey: currentPubkey,
      );

      // Mentioning humans outside the channel prompts "Invite" / "Do
      // nothing" (send without inviting) — mirrors desktop's
      // NonMemberMentionDialog. Agents keep the existing silent auto-add.
      if (scan.humans.isNotEmpty) {
        if (!context.mounted) return;
        final choice = await _promptNonMemberMention(
          context,
          names: [for (final candidate in scan.humans) candidate.label],
          canInvite: scan.canAddMembers,
        );
        if (choice == null) return; // Dismissed — keep the draft, send nothing.
        outgoing.resolveHumanChoice(choice, scan.humans);
      }

      final queuedAttachments = List<_PendingAttachment>.of(attachments.value);
      final channelActions = ref.read(channelActionsProvider);

      // An add that was refused doesn't block the message: it is reported and
      // the un-added mentions are demoted to reference tags so the send lands.
      Future<void> addMentionedNonMembers() => outgoing.addNonMembers(
        channelActions,
        scan: scan,
        messenger: messenger,
      );

      isSending.value = true;
      try {
        if (queuedAttachments.isEmpty) {
          if (!context.mounted) return;
          await _sendTextOnlyDraft(
            context: context,
            controller: controller,
            mentionMap: mentionMap,
            draftRevision: draftRevision,
            submittedDraftRevision: submittedDraftRevision,
            focusNode: focusNode,
            clearComposer: clearComposer,
            addMentionedNonMembers: addMentionedNonMembers,
            payload: _ComposeDraftPayload.fromDraft(
              text: text,
              attachments: const [],
              customEmoji: customEmoji,
            ),
            outgoing: outgoing,
            onSend: onSend,
            messenger: messenger,
          );
          return;
        }

        final draftText = controller.value;
        final draftAttachments = List<_PendingAttachment>.of(attachments.value);
        final draftMentions = Map<String, MentionCandidate>.of(
          mentionMap.value,
        );
        clearComposer();
        final clearedDraftRevision = draftRevision.value;
        uploadingCount.value += 1;
        uploadProgress.value = 0;
        isSending.value = false;
        final queueGeneration = uploadGeneration.value;
        final cancellation = UploadCancellationToken();
        final uploadService = ref.read(mediaUploadServiceProvider);
        activeUploadCancellation.value = cancellation;
        final delivery = onSend;
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
              controller.value = draftText;
              attachments.value = draftAttachments;
              retainedForRetry = true;
              mentionMap.value
                ..clear()
                ..addAll(draftMentions);
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

    final queueAttachment = useCallback(
      (
        XFile file,
        _PendingAttachmentKind kind, {
        bool deleteAfterUse = false,
      }) => _queueComposerAttachment(
        file,
        kind,
        voiceNoteRef,
        attachments,
        uploadError,
        draftRevision,
        deleteAfterUse: deleteAfterUse,
      ),
      [voiceNoteRef, draftRevision, uploadError, attachments],
    );
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
        _queueComposerImages(
          images,
          voiceNote,
          attachments,
          uploadError,
          draftRevision,
          deleteAfterUse,
        );

    Future<void> retainAndQueueImages(List<XFile> images) =>
        _retainAndQueueImages(context, images, queueImages);

    final pasteClipboardImage = useCallback(() {
      ContextMenuController.removeAny();
      unawaited(() async {
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
      }());
    }, [context, ref, queueAttachment, uploadError]);

    final buildContextMenu = useCallback<EditableTextContextMenuBuilder>((
      context,
      editableTextState,
    ) {
      if (defaultTargetPlatform == TargetPlatform.iOS &&
          SystemContextMenu.isSupportedByField(editableTextState)) {
        return SystemContextMenu.editableText(
          editableTextState: editableTextState,
          items: [
            if (clipboardHasImage.value)
              IOSSystemContextMenuItemCustom(
                title: 'Paste Image',
                onPressed: pasteClipboardImage,
              ),
            ...SystemContextMenu.getDefaultItems(editableTextState),
          ],
        );
      }

      final buttonItems = [...editableTextState.contextMenuButtonItems];
      if (defaultTargetPlatform == TargetPlatform.iOS &&
          clipboardHasImage.value) {
        buttonItems.insert(
          0,
          ContextMenuButtonItem(
            label: 'Paste Image',
            onPressed: pasteClipboardImage,
          ),
        );
      }
      return AdaptiveTextSelectionToolbar.buttonItems(
        anchors: editableTextState.contextMenuAnchors,
        buttonItems: buttonItems,
      );
    }, [clipboardHasImage, pasteClipboardImage]);

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

    // Wrap (or insert) markdown formatting around the current selection.
    void applyFormat(String prefix, [String? suffix]) {
      suffix ??= prefix;
      final text = controller.text;
      final sel = controller.selection;
      if (!sel.isValid) return;

      isModifyingText.value = true;
      try {
        if (sel.isCollapsed) {
          final offset = sel.baseOffset;
          final updated =
              '${text.substring(0, offset)}$prefix$suffix${text.substring(offset)}';
          controller.text = updated;
          controller.selection = TextSelection.collapsed(
            offset: offset + prefix.length,
          );
        } else {
          final selected = text.substring(sel.start, sel.end);
          final updated =
              '${text.substring(0, sel.start)}$prefix$selected$suffix${text.substring(sel.end)}';
          controller.text = updated;
          controller.selection = TextSelection.collapsed(
            offset: sel.start + prefix.length + selected.length + suffix.length,
          );
        }
      } finally {
        isModifyingText.value = false;
      }
      focusNode.requestFocus();
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
            choose,
            errorMessage: errorMessage,
          );

    void toggleAttachments() {
      attachmentSurface.value = switch (attachmentSurface.value) {
        _AttachmentSurface.closed => _AttachmentSurface.menu,
        _AttachmentSurface.menu => _AttachmentSurface.closed,
        _AttachmentSurface.camera ||
        _AttachmentSurface.photos => _AttachmentSurface.menu,
      };
    }

    void handleAttachmentTap(BuildContext triggerContext) {
      if (defaultTargetPlatform != TargetPlatform.iOS ||
          attachmentSurface.value != _AttachmentSurface.closed) {
        toggleAttachments();
        return;
      }

      unawaited(
        iosAttachmentPopover
            .present(
              sourceContext: triggerContext,
              onCapture: (image) => retainAndQueueImages([image]),
              onChoosePhotos: retainAndQueueImages,
              onAllPhotos: () => chooseAttachment(() async {
                final photos = await ref
                    .read(mediaUploadServiceProvider)
                    .pickGalleryImages();
                queueImages(photos);
              }, errorMessage: 'Unable to open your photo library.'),
              onVideo: () => chooseAttachment(() {
                final service = ref.read(mediaUploadServiceProvider);
                return pickThenQueue(
                  pick: service.pickGalleryVideo,
                  kind: _PendingAttachmentKind.video,
                );
              }),
              onVoiceNote: voiceNote.start,
              onFiles: () => chooseAttachment(() {
                final service = ref.read(mediaUploadServiceProvider);
                return pickThenQueue(
                  pick: service.pickAttachmentFile,
                  kind: _PendingAttachmentKind.file,
                );
              }),
            )
            .then((didPresent) {
              if (!didPresent && context.mounted) {
                focusNode.unfocus();
                toggleAttachments();
              }
            }),
      );
    }

    void openCamera() {
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
    final suggestionOverlayController = useMemoized(
      OverlayPortalController.new,
    );

    useEffect(() {
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (context.mounted) suggestionOverlayController.show();
      });
      return null;
    }, [suggestionOverlayController]);

    void expandComposer() => _expandComposer(
      context: context,
      isExpanded: isComposerExpanded,
      attachmentSurface: attachmentSurface,
      onFocusRequested: onFocusRequested,
      focusNode: focusNode,
      view: appView,
      androidImeTransitionStarted: androidImeTransitionStarted,
      androidImeFallbackTimer: androidImeFallbackTimer,
    );

    _useComposerFocusRestorer(
      onChanged: onFocusRestorerChanged,
      isExpanded: isComposerExpanded,
      focusNode: focusNode,
      expand: expandComposer,
    );

    final suggestionPanel = _composerSuggestionPanel(
      channelSuggestions: channelSuggestions,
      mentionSuggestions: suggestions,
      userCache: userCache,
      currentPubkey: currentPubkey,
      isDmChannel: isDmChannel,
      onChannelSelect: insertChannel,
      onMentionSelect: insertMention,
    );
    Widget buildOverlayPanel(_AttachmentSurface surface) {
      return _composerAttachmentPanel(
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
        onVoiceNote: voiceNote.start,
        onFiles: () => chooseAttachment(() {
          final service = ref.read(mediaUploadServiceProvider);
          return pickThenQueue(
            pick: service.pickAttachmentFile,
            kind: _PendingAttachmentKind.file,
          );
        }),
        onCapture: (image) async {
          attachmentSurface.value = _AttachmentSurface.closed;
          await retainAndQueueImages([image]);
        },
        onPickAllPhotos: ref.read(mediaUploadServiceProvider).pickGalleryImages,
        onChoosePhotos: (photos) async {
          attachmentSurface.value = _AttachmentSurface.closed;
          await retainAndQueueImages(photos);
        },
        onChooseAllPhotos: (photos) async {
          attachmentSurface.value = _AttachmentSurface.closed;
          queueImages(photos);
        },
      );
    }

    // Suggestions and attachments live in the overlay.
    final hasPendingUploads = uploadingCount.value > 0;
    return _ComposerDockFrame(
      expansionAnimation: composerExpansionController,
      forceFullWidth: _voiceNoteFullWidth(voiceNote, attachments.value),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          _UploadProgressMotion(
            visible: hasPendingUploads,
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
            attachmentSurface: attachmentSurface,
            reducedMotion: reducedMotion,
            buildOverlayPanel: buildOverlayPanel,
            onDismissAttachmentSurface: () {
              attachmentSurface.value = _AttachmentSurface.closed;
            },
            child: _ComposeBarLayout(
              voiceNoteRecorder: voiceNote.recorder,
              attachments: attachments.value,
              onRemoveAttachment: removeAttachment,
              uploadError: uploadError.value,
              isExpanded: isComposerExpanded.value,
              controller: controller,
              focusNode: focusNode,
              contextMenuBuilder: buildContextMenu,
              onContentInserted: uploadPastedImage,
              onSend: () => unawaited(send()),
              resolvedHint: resolvedHint,
              attachmentSurface: attachmentSurface.value,
              onAttachmentTap: handleAttachmentTap,
              onExpand: expandComposer,
              expansionAnimation: composerExpansionController,
              formattingOpen: showFormatting.value,
              onCloseFormatting: () => showFormatting.value = false,
              motionDuration: motionDuration,
              resizeDuration: resizeDuration,
              onFormat: applyFormat,
              onMention: () {
                attachmentSurface.value = _AttachmentSurface.closed;
                triggerMention();
              },
              onChannel: () {
                attachmentSurface.value = _AttachmentSurface.closed;
                triggerChannel();
              },
              onEmoji: () {
                attachmentSurface.value = _AttachmentSurface.closed;
                isEmojiPickerOpen.value = true;
                _showComposerEmojiPicker(context, insertEmoji, () {
                  if (!context.mounted) return;
                  isEmojiPickerOpen.value = false;
                  focusNode.requestFocus();
                });
              },
              onOpenFormatting: () {
                attachmentSurface.value = _AttachmentSurface.closed;
                showFormatting.value = true;
              },
              hasPendingUploads: hasPendingUploads,
              canSend: composerText.trim().isNotEmpty || hasAttachments,
              isSending: isSending.value,
            ),
          ),
        ],
      ),
    );
  }
}
