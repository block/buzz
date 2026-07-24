import 'dart:async';
import 'dart:collection';

import 'package:camera/camera.dart' as camera;
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/physics.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:image_picker/image_picker.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import 'package:nostr/nostr.dart' as nostr;

import '../../shared/relay/relay.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/avatar_image.dart';
import '../profile/user_cache_provider.dart';
import '../profile/user_profile.dart';
import '../custom_emoji/custom_emoji.dart';
import '../custom_emoji/custom_emoji_provider.dart';
import 'channel.dart';
import 'channel_management_provider.dart';
import 'channels_provider.dart';
import 'emoji_picker.dart';
import 'mentions/mention_candidates.dart';
import 'mentions/mention_candidates_provider.dart';
import 'mentions/mention_ranking.dart';

part 'compose_bar/helpers.dart';
part 'compose_bar/suggestions.dart';
part 'compose_bar/formatting_toolbar.dart';
part 'compose_bar/attachments.dart';
part 'compose_bar/camera_preview.dart';
part 'compose_bar/send_button.dart';

const _pastedImageMimeTypes = <String>[
  'image/jpeg',
  'image/jpg',
  'image/png',
  'image/webp',
];

/// Rich compose bar with @mention autocomplete, emoji picker, and a markdown
/// formatting toolbar. Used in both channel and thread views — the caller
/// provides an [onSend] callback that handles actual message submission.
typedef ComposeBarOnSend =
    Future<void> Function(
      String content,
      List<String> mentionPubkeys, {
      List<List<String>> mediaTags,
    });

class ComposeBar extends HookConsumerWidget {
  final String channelId;
  final String channelName;
  final String? hintText;
  final ComposeBarOnSend onSend;

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
    required this.onSend,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final controller = useTextEditingController();
    final focusNode = useFocusNode();
    final isComposerExpanded = useState(false);
    final showAttachments = useState(false);
    final showCamera = useState(false);
    final isSending = useState(false);
    final showFormatting = useState(false);
    final attachments = useState<List<BlobDescriptor>>([]);
    final uploadError = useState<String?>(null);
    final uploadingCount = useState(0);
    final clipboardHasImage = useState(false);
    final hasAttachments = attachments.value.isNotEmpty;
    final hasPendingUploads = uploadingCount.value > 0;
    final customEmoji = ref.watch(customEmojiListProvider);
    final reducedMotion = MediaQuery.disableAnimationsOf(context);
    final composerExpansionController = useAnimationController(
      initialValue: 0,
      upperBound: 1.05,
    );
    final composerExpansionValue = useAnimation(composerExpansionController);
    final composerExpansionProgress = composerExpansionValue
        .clamp(0.0, 1.0)
        .toDouble();

    final resolvedHint =
        hintText ??
        (channelName.isNotEmpty ? 'Message #$channelName' : 'Message\u2026');

    useEffect(() {
      final target = isComposerExpanded.value ? 1.0 : 0.0;
      if (reducedMotion) {
        composerExpansionController.value = target;
      } else if ((composerExpansionController.value - target).abs() > 0.001) {
        composerExpansionController.animateWith(
          SpringSimulation(
            SpringDescription.withDurationAndBounce(
              duration: const Duration(milliseconds: 280),
              bounce: 0.16,
            ),
            composerExpansionController.value,
            target,
            0,
            snapToEnd: true,
          ),
        );
      }
      return null;
    }, [isComposerExpanded.value, reducedMotion]);

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

    final membersAsync = ref.watch(channelMembersProvider(channelId));
    final currentPubkey = ref.watch(currentPubkeyProvider);
    final userCache = ref.watch(userCacheProvider);
    final isDmChannel =
        channelsAsync.asData?.value.any((c) => c.id == channelId && c.isDm) ??
        false;

    // Preload profiles for channel members, mentionable agents, and their
    // owners so @mention suggestions show names ("managed by …" included).
    final relayAgents = ref.watch(agentDirectoryProvider).asData?.value;
    final agentOwners = ref.watch(agentOwnersProvider).asData?.value;
    useEffect(
      () {
        final memberList = membersAsync.asData?.value ?? <ChannelMember>[];
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
        relayAgents?.length,
        agentOwners?.length,
      ],
    );

    // Typing indicator broadcast — throttled to one event per 3 seconds.
    final lastTypingSentMs = useRef(0);
    final isModifyingText = useRef(false);

    // Detect @mention query and broadcast typing on text / selection change.
    useEffect(() {
      void listener() {
        if (isModifyingText.value) return;
        final text = controller.text;
        final sel = controller.selection;

        // Broadcast typing indicator (throttled).
        if (text.isNotEmpty) {
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
      spliceAndMoveCursor(
        controller,
        focusNode,
        start: start,
        replacement: '@$name ',
      );
      mentionQuery.value = null;
    }

    // Insert a selected channel into the text field.
    void insertChannel(Channel channel) {
      final start = channelStartIdx.value.clamp(0, controller.text.length);
      spliceAndMoveCursor(
        controller,
        focusNode,
        start: start,
        replacement: '#${channel.name} ',
      );
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
      controller.clear();
      attachments.value = [];
      mentionMap.value.clear();
      mentionQuery.value = null;
      channelQuery.value = null;
      showAttachments.value = false;
      showCamera.value = false;
      showFormatting.value = false;
      uploadError.value = null;
      focusNode.requestFocus();
    }

    void removeAttachment(String url) {
      attachments.value = _withoutAttachment(attachments.value, url);
    }

    // Send the message.
    Future<void> send() async {
      final text = controller.text.trim();
      if ((text.isEmpty && !hasAttachments) ||
          isSending.value ||
          hasPendingUploads) {
        return;
      }

      // Extract pubkeys for mentions present in the final text.
      final selectedMentions = <MentionCandidate>[
        for (final entry in mentionMap.value.entries)
          if (hasMention(text, entry.key)) entry.value,
      ];
      final pubkeys = LinkedHashSet<String>.from(
        selectedMentions.map((candidate) => candidate.pubkey.toLowerCase()),
      ).toList();
      final nonMemberAgentPubkeys = <String>[];
      final nonMemberHumans = <MentionCandidate>[];
      if (selectedMentions.isNotEmpty) {
        final currentChannel = (await ref.read(
          channelsProvider.future,
        )).firstWhere((channel) => channel.id == channelId);
        if (!currentChannel.isDm) {
          final memberPubkeys = (await ref.read(
            channelMembersProvider(channelId).future,
          )).map((member) => member.pubkey.toLowerCase()).toSet();
          final seenNonMembers = <String>{};
          for (final candidate in selectedMentions) {
            final pk = candidate.pubkey.toLowerCase();
            if (memberPubkeys.contains(pk)) continue;
            if (!seenNonMembers.add(pk)) continue;
            if (candidate.isAgent) {
              nonMemberAgentPubkeys.add(pk);
            } else {
              nonMemberHumans.add(candidate);
            }
          }
        }
      }

      // Mentioning humans outside the channel prompts "Invite" / "Do
      // nothing" (send without inviting) — mirrors desktop's
      // NonMemberMentionDialog. Agents keep the existing silent auto-add.
      var mentionPubkeys = pubkeys;
      final referenceMentionTags = <List<String>>[];
      var inviteHumanPubkeys = const <String>[];
      if (nonMemberHumans.isNotEmpty) {
        if (!context.mounted) return;
        final choice = await _promptNonMemberMention(
          context,
          names: [for (final candidate in nonMemberHumans) candidate.label],
        );
        switch (choice) {
          case null:
            return; // Dismissed — keep the draft, send nothing.
          case _NonMemberMentionChoice.invite:
            inviteHumanPubkeys = [
              for (final candidate in nonMemberHumans)
                candidate.pubkey.toLowerCase(),
            ];
          case _NonMemberMentionChoice.sendWithoutInviting:
            // Strip their p-tags (no channel notification) but keep a
            // `mention` reference tag so their name still renders —
            // mirrors desktop's mergeOutgoingTagsWithReferenceMentions.
            final excluded = {
              for (final candidate in nonMemberHumans)
                candidate.pubkey.toLowerCase(),
            };
            mentionPubkeys = [
              for (final pk in pubkeys)
                if (!excluded.contains(pk)) pk,
            ];
            referenceMentionTags.addAll([
              for (final pk in excluded) ['mention', pk],
            ]);
        }
      }

      final payload = _ComposeDraftPayload.fromDraft(
        text: text,
        attachments: attachments.value,
        customEmoji: customEmoji,
      );

      isSending.value = true;
      try {
        if (nonMemberAgentPubkeys.isNotEmpty) {
          await ref
              .read(channelActionsProvider)
              .addMembers(
                channelId: channelId,
                pubkeys: nonMemberAgentPubkeys,
                role: 'bot',
              );
        }
        if (inviteHumanPubkeys.isNotEmpty) {
          await ref
              .read(channelActionsProvider)
              .addMembers(channelId: channelId, pubkeys: inviteHumanPubkeys);
        }
        await onSend(
          payload.content,
          mentionPubkeys,
          mediaTags: [...payload.mediaTags, ...referenceMentionTags],
        );
        if (context.mounted) {
          clearComposer();
        }
      } finally {
        if (context.mounted) isSending.value = false;
      }
    }

    Future<void> pickAndUpload(Future<BlobDescriptor?> Function() pick) async {
      uploadError.value = null;
      uploadingCount.value += 1;
      try {
        final uploaded = await pick();
        if (uploaded != null && context.mounted) {
          attachments.value = [...attachments.value, uploaded];
        }
      } catch (error) {
        if (context.mounted) {
          uploadError.value = _formatUploadError(error);
        }
      } finally {
        if (context.mounted) {
          uploadingCount.value -= 1;
        }
      }
    }

    Widget buildContextMenu(
      BuildContext context,
      EditableTextState editableTextState,
    ) {
      void pasteImage() {
        ContextMenuController.removeAny();
        pickAndUpload(
          ref.read(mediaUploadServiceProvider).readAndUploadClipboardImage,
        );
      }

      if (defaultTargetPlatform == TargetPlatform.iOS &&
          SystemContextMenu.isSupportedByField(editableTextState)) {
        return SystemContextMenu.editableText(
          editableTextState: editableTextState,
          items: [
            if (clipboardHasImage.value)
              IOSSystemContextMenuItemCustom(
                title: 'Paste Image',
                onPressed: pasteImage,
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
          ContextMenuButtonItem(label: 'Paste Image', onPressed: pasteImage),
        );
      }
      return AdaptiveTextSelectionToolbar.buttonItems(
        anchors: editableTextState.contextMenuAnchors,
        buttonItems: buttonItems,
      );
    }

    void uploadPastedImage(KeyboardInsertedContent content) {
      final bytes = content.data;
      if (bytes == null || bytes.isEmpty) {
        uploadError.value = 'Unable to read pasted image';
        return;
      }

      pickAndUpload(
        () => ref
            .read(mediaUploadServiceProvider)
            .uploadImage(XFile.fromData(bytes)),
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

    // ----- Widget tree ----------------------------------------------------

    void chooseAttachment(Future<BlobDescriptor?> Function() pick) {
      showAttachments.value = false;
      showCamera.value = false;
      pickAndUpload(pick);
    }

    void toggleAttachments() {
      if (showCamera.value) {
        showCamera.value = false;
        showAttachments.value = false;
        return;
      }
      showCamera.value = false;
      showAttachments.value = !showAttachments.value;
    }

    void openCamera() {
      focusNode.unfocus();
      showAttachments.value = false;
      showCamera.value = true;
    }

    final motionDuration = reducedMotion
        ? Duration.zero
        : const Duration(milliseconds: 180);
    final suggestionOverlayController = useMemoized(
      OverlayPortalController.new,
    );

    useEffect(() {
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (context.mounted) suggestionOverlayController.show();
      });
      return null;
    }, [suggestionOverlayController]);

    void expandComposer() {
      if (isComposerExpanded.value) return;
      showAttachments.value = false;
      showCamera.value = false;
      isComposerExpanded.value = true;
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (context.mounted) focusNode.requestFocus();
      });
    }

    final suggestionPanel = channelSuggestions.isNotEmpty
        ? KeyedSubtree(
            key: const ValueKey('channel-suggestions'),
            child: _ChannelSuggestions(
              suggestions: channelSuggestions,
              onSelect: insertChannel,
            ),
          )
        : suggestions.isNotEmpty
        ? KeyedSubtree(
            key: const ValueKey('mention-suggestions'),
            child: _MentionSuggestions(
              suggestions: suggestions,
              userCache: userCache,
              currentPubkey: currentPubkey,
              isDmChannel: isDmChannel,
              onSelect: insertMention,
            ),
          )
        : const SizedBox.shrink(key: ValueKey('no-suggestions'));
    final overlayPanel = showCamera.value
        ? KeyedSubtree(
            key: const ValueKey('camera-preview'),
            child: _InlineCameraPreview(
              onClose: () => showCamera.value = false,
              onCapture: (image) async {
                await pickAndUpload(
                  () => ref.read(mediaUploadServiceProvider).uploadImage(image),
                );
                if (context.mounted) showCamera.value = false;
              },
            ),
          )
        : showAttachments.value
        ? KeyedSubtree(
            key: const ValueKey('attachment-menu'),
            child: Align(
              alignment: Alignment.bottomLeft,
              heightFactor: 1,
              child: _AttachmentMenu(
                onCamera: openCamera,
                onPhotos: () => chooseAttachment(
                  ref.read(mediaUploadServiceProvider).pickAndUploadImage,
                ),
                onVideo: () => chooseAttachment(
                  ref.read(mediaUploadServiceProvider).pickAndUploadVideo,
                ),
                onFiles: () => chooseAttachment(
                  ref.read(mediaUploadServiceProvider).pickAndUploadFile,
                ),
              ),
            ),
          )
        : suggestionPanel;

    // Suggestions and attachments live in the overlay so showing them cannot
    // reflow the composer. Both stay anchored just above the capsule.
    return Padding(
      padding: EdgeInsets.only(
        left: Grid.twelve,
        right: Grid.twelve,
        bottom: MediaQuery.viewPaddingOf(context).bottom + Grid.xxs,
      ),
      child: OverlayPortal.overlayChildLayoutBuilder(
        controller: suggestionOverlayController,
        overlayChildBuilder: (context, layoutInfo) {
          final composerOrigin = MatrixUtils.transformPoint(
            layoutInfo.childPaintTransform,
            Offset.zero,
          );
          return Positioned(
            left: composerOrigin.dx,
            bottom: layoutInfo.overlaySize.height - composerOrigin.dy,
            width: layoutInfo.childSize.width,
            child: ClipRect(
              child: Padding(
                padding: const EdgeInsets.only(bottom: Grid.xxs),
                child: _SuggestionPanelMotion(
                  duration: motionDuration,
                  child: overlayPanel,
                ),
              ),
            ),
          );
        },
        child: Container(
          decoration: BoxDecoration(
            color: context.colors.surfaceContainerHighest,
            borderRadius: BorderRadius.circular(Radii.dialog),
            border: Border.all(
              color: Colors.black.withValues(alpha: 0.04),
              width: 1,
            ),
          ),
          padding: const EdgeInsets.all(Grid.xxs),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              if (hasAttachments || hasPendingUploads) ...[
                _AttachmentStrip(
                  attachments: attachments.value,
                  uploadingCount: uploadingCount.value,
                  onRemove: removeAttachment,
                ),
                const SizedBox(height: Grid.xxs),
              ],

              if (uploadError.value case final error?) ...[
                Align(
                  alignment: Alignment.centerLeft,
                  child: Text(
                    error,
                    style: context.textTheme.bodySmall?.copyWith(
                      color: context.colors.error,
                    ),
                  ),
                ),
                const SizedBox(height: Grid.xxs),
              ],

              // Keep the default state out of the focus system entirely so
              // restored native focus cannot expand a newly opened channel.
              if (isComposerExpanded.value)
                TextField(
                  controller: controller,
                  focusNode: focusNode,
                  textInputAction: TextInputAction.send,
                  contextMenuBuilder: buildContextMenu,
                  contentInsertionConfiguration: ContentInsertionConfiguration(
                    allowedMimeTypes: _pastedImageMimeTypes,
                    onContentInserted: uploadPastedImage,
                  ),
                  onSubmitted: (_) => send(),
                  minLines: 1,
                  maxLines: 5,
                  style: context.textTheme.bodyLarge,
                  decoration: InputDecoration(
                    hintText: resolvedHint,
                    hintStyle: context.textTheme.bodyLarge?.copyWith(
                      color: context.colors.onSurfaceVariant,
                    ),
                    border: InputBorder.none,
                    enabledBorder: InputBorder.none,
                    focusedBorder: InputBorder.none,
                    contentPadding: const EdgeInsets.symmetric(
                      horizontal: Grid.half,
                      vertical: Grid.half,
                    ),
                    isDense: true,
                  ),
                )
              else
                Row(
                  children: [
                    _AttachmentTrigger(
                      open: showAttachments.value || showCamera.value,
                      onTap: toggleAttachments,
                    ),
                    const SizedBox(width: Grid.xxs),
                    Expanded(
                      child: Semantics(
                        button: true,
                        label: resolvedHint,
                        child: GestureDetector(
                          behavior: HitTestBehavior.opaque,
                          onTap: expandComposer,
                          child: Padding(
                            padding: const EdgeInsets.symmetric(
                              vertical: Grid.half,
                            ),
                            child: Align(
                              alignment: Alignment.centerLeft,
                              child: Text(
                                resolvedHint,
                                style: context.textTheme.bodyLarge?.copyWith(
                                  color: context.colors.onSurfaceVariant,
                                ),
                              ),
                            ),
                          ),
                        ),
                      ),
                    ),
                  ],
                ),

              ClipRect(
                child: Align(
                  alignment: Alignment.topCenter,
                  heightFactor: composerExpansionValue,
                  child: IgnorePointer(
                    ignoring: composerExpansionValue < 0.98,
                    child: Opacity(
                      opacity: composerExpansionProgress,
                      child: Transform.translate(
                        offset: Offset(
                          0,
                          Grid.xxs * (1 - composerExpansionProgress),
                        ),
                        child: Column(
                          children: [
                            const SizedBox(height: Grid.xxs),
                            Row(
                              children: [
                                _AttachmentTrigger(
                                  open:
                                      showAttachments.value ||
                                      showCamera.value ||
                                      showFormatting.value,
                                  onTap: () {
                                    if (showFormatting.value) {
                                      showFormatting.value = false;
                                    } else {
                                      toggleAttachments();
                                    }
                                  },
                                ),
                                const SizedBox(width: Grid.half),
                                Expanded(
                                  child: AnimatedSwitcher(
                                    duration: motionDuration,
                                    switchInCurve: Curves.easeOutCubic,
                                    switchOutCurve: Curves.easeInCubic,
                                    layoutBuilder:
                                        (currentChild, previousChildren) =>
                                            Stack(
                                              alignment: Alignment.centerLeft,
                                              children: [
                                                ...previousChildren,
                                                ?currentChild,
                                              ],
                                            ),
                                    child: showFormatting.value
                                        ? _FormattingToolbar(
                                            onFormat: applyFormat,
                                          )
                                        : Row(
                                            key: const ValueKey(
                                              'standard-actions',
                                            ),
                                            children: [
                                              _ComposeAction(
                                                icon: LucideIcons.atSign,
                                                onTap: () {
                                                  showAttachments.value = false;
                                                  showCamera.value = false;
                                                  triggerMention();
                                                },
                                              ),
                                              _ComposeAction(
                                                icon: LucideIcons.hash,
                                                onTap: () {
                                                  showAttachments.value = false;
                                                  showCamera.value = false;
                                                  triggerChannel();
                                                },
                                              ),
                                              _ComposeAction(
                                                icon: LucideIcons.smilePlus,
                                                onTap: () {
                                                  showAttachments.value = false;
                                                  showCamera.value = false;
                                                  showEmojiPicker(
                                                    context: context,
                                                    onSelect: insertEmoji,
                                                  );
                                                },
                                              ),
                                              _ComposeAction(
                                                icon: LucideIcons.aLargeSmall,
                                                onTap: () {
                                                  showAttachments.value = false;
                                                  showCamera.value = false;
                                                  showFormatting.value = true;
                                                },
                                              ),
                                              const Spacer(),
                                              _SendButton(
                                                isDisabled: hasPendingUploads,
                                                isSending: isSending.value,
                                                onTap: send,
                                              ),
                                            ],
                                          ),
                                  ),
                                ),
                              ],
                            ),
                          ],
                        ),
                      ),
                    ),
                  ),
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}
