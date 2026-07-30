import 'dart:async';
import 'dart:collection';
import 'dart:math' as math;

import 'package:camera/camera.dart' as camera;
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/physics.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:image_picker/image_picker.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import 'package:nostr/nostr.dart' as nostr;

import '../../shared/relay/relay.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/avatar_image.dart';
import '../../shared/widgets/buzz_loading_indicator.dart';
import '../../shared/widgets/keyboard_dismiss_on_drag.dart';
import '../profile/user_cache_provider.dart';
import '../profile/user_profile.dart';
import '../../shared/custom_emoji/custom_emoji.dart';
import '../../shared/custom_emoji/custom_emoji_provider.dart';
import '../activity/compose_drafts_provider.dart';
import 'camera_capture_cleanup.dart';
import 'channel.dart';
import 'channel_management_provider.dart';
import 'channels_provider.dart';
import 'emoji_picker.dart';
import 'mentions/mention_candidates.dart';
import 'mentions/mention_candidates_provider.dart';
import 'mentions/mention_ranking.dart';
import 'photo_library.dart';

part 'compose_bar/helpers.dart';
part 'compose_bar/markdown_editing_controller.dart';
part 'compose_bar/suggestions.dart';
part 'compose_bar/formatting_toolbar.dart';
part 'compose_bar/attachments.dart';
part 'compose_bar/photo_gallery_picker.dart';
part 'compose_bar/ios_photo_picker.dart';
part 'compose_bar/ios_attachment_popover.dart';
part 'compose_bar/camera_preview.dart';
part 'compose_bar/send_button.dart';
part 'compose_bar/layout.dart';

const _maxConcurrentImageUploads = 3;

/// Rich compose bar with @mention autocomplete and a markdown formatting
/// toolbar. Used in both channel and thread views — the caller provides an
/// [onSend] callback that handles actual message submission.
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
    final controller = useMemoized(_MarkdownEditingController.new);
    useEffect(() => controller.dispose, [controller]);

    // Restore and persist unsent text as a local draft so the Activity
    // inbox Drafts filter reflects real composer state.
    //
    // The effect is additionally keyed on the active relay + pubkey identity:
    // provider-level namespacing alone cannot protect a composer that stays
    // mounted through an in-place community/account switch — the controller
    // would retain the old identity's text and the next edit would persist it
    // into the new identity's store. On identity change we replace the
    // controller content with the new identity's own saved draft (or clear).
    final draftKey = composeDraftKey(channelId, threadHeadId: threadHeadId);
    final draftIdentity =
        '${ref.watch(relayConfigProvider).baseUrl}'
        ':${ref.watch(myPubkeyProvider) ?? 'anon'}';
    final lastDraftIdentity = useRef<String?>(null);
    useEffect(() {
      final identityChanged =
          lastDraftIdentity.value != null &&
          lastDraftIdentity.value != draftIdentity;
      lastDraftIdentity.value = draftIdentity;
      final saved = ref.read(composeDraftsProvider.notifier).textFor(draftKey);
      if (identityChanged) {
        controller.text = saved ?? '';
      } else if (saved != null && controller.text.isEmpty) {
        controller.text = saved;
      }
      void persistDraft() {
        ref
            .read(composeDraftsProvider.notifier)
            .save(
              key: draftKey,
              channelId: channelId,
              threadHeadId: threadHeadId,
              text: controller.text,
            );
      }

      controller.addListener(persistDraft);
      return () => controller.removeListener(persistDraft);
    }, [controller, draftKey, draftIdentity]);
    final focusNode = useFocusNode();
    final isComposerExpanded = useState(false);
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
      attachmentSurface.value = _AttachmentSurface.closed;
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

    Future<void> pickThenUpload({
      required Future<XFile?> Function() pick,
      required Future<BlobDescriptor> Function(XFile file) upload,
    }) async {
      uploadError.value = null;
      try {
        final picked = await pick();
        if (picked == null || !context.mounted) return;
        await pickAndUpload(() => upload(picked));
      } catch (error) {
        if (context.mounted) {
          uploadError.value = _formatUploadError(error);
        }
      }
    }

    Future<void> uploadImages(List<XFile> images) async {
      if (images.isEmpty) return;
      uploadError.value = null;
      uploadingCount.value += images.length;
      try {
        Future<({BlobDescriptor? uploaded, Object? error})> uploadImage(
          XFile image,
        ) async {
          try {
            final uploaded = await ref
                .read(mediaUploadServiceProvider)
                .uploadImage(image);
            return (uploaded: uploaded, error: null);
          } catch (error) {
            return (uploaded: null, error: error);
          }
        }

        final results = <({BlobDescriptor? uploaded, Object? error})>[];
        for (
          var start = 0;
          start < images.length;
          start += _maxConcurrentImageUploads
        ) {
          final end = math.min(
            start + _maxConcurrentImageUploads,
            images.length,
          );
          results.addAll(
            await Future.wait([
              for (final image in images.sublist(start, end))
                uploadImage(image),
            ]),
          );
        }
        if (!context.mounted) return;

        final uploaded = [for (final result in results) ?result.uploaded];
        if (uploaded.isNotEmpty) {
          attachments.value = [...attachments.value, ...uploaded];
        }
        final firstError = results
            .map((result) => result.error)
            .whereType<Object>()
            .firstOrNull;
        if (firstError != null) {
          uploadError.value = _formatUploadError(firstError);
        }
      } finally {
        if (context.mounted) {
          uploadingCount.value = math.max(
            0,
            uploadingCount.value - images.length,
          );
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

    void chooseAttachment(
      Future<void> Function() choose, {
      String? errorMessage,
    }) {
      attachmentSurface.value = _AttachmentSurface.closed;
      unawaited(() async {
        try {
          await choose();
        } catch (error) {
          if (context.mounted) {
            uploadError.value = errorMessage ?? _formatUploadError(error);
          }
        }
      }());
    }

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
              onCapture: (image) => pickAndUpload(
                () => ref.read(mediaUploadServiceProvider).uploadImage(image),
              ),
              onChoosePhotos: uploadImages,
              onAllPhotos: () => chooseAttachment(() async {
                final photos = await ref
                    .read(mediaUploadServiceProvider)
                    .pickGalleryImages();
                await uploadImages(photos);
              }, errorMessage: 'Unable to open your photo library.'),
              onVideo: () => chooseAttachment(() {
                final service = ref.read(mediaUploadServiceProvider);
                return pickThenUpload(
                  pick: service.pickGalleryVideo,
                  upload: service.uploadVideo,
                );
              }),
              onFiles: () => chooseAttachment(() {
                final service = ref.read(mediaUploadServiceProvider);
                return pickThenUpload(
                  pick: service.pickAttachmentFile,
                  upload: service.uploadFile,
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

    final motionDuration = reducedMotion
        ? Duration.zero
        : Duration(
            milliseconds:
                attachmentSurface.value == _AttachmentSurface.camera ||
                    attachmentSurface.value == _AttachmentSurface.photos
                ? 320
                : 250,
          );
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
      attachmentSurface.value = _AttachmentSurface.closed;
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
    Widget buildOverlayPanel(_AttachmentSurface surface) {
      return _AttachmentSurfacePanel(
        key: ValueKey(
          surface == _AttachmentSurface.closed
              ? 'composer-suggestions'
              : 'attachment-surface',
        ),
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
          return pickThenUpload(
            pick: service.pickGalleryVideo,
            upload: service.uploadVideo,
          );
        }),
        onFiles: () => chooseAttachment(() {
          final service = ref.read(mediaUploadServiceProvider);
          return pickThenUpload(
            pick: service.pickAttachmentFile,
            upload: service.uploadFile,
          );
        }),
        onCapture: (image) async {
          attachmentSurface.value = _AttachmentSurface.closed;
          await pickAndUpload(
            () => ref.read(mediaUploadServiceProvider).uploadImage(image),
          );
        },
        onPickAllPhotos: ref.read(mediaUploadServiceProvider).pickGalleryImages,
        onChoosePhotos: (photos) async {
          attachmentSurface.value = _AttachmentSurface.closed;
          await uploadImages(photos);
        },
      );
    }

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
          return ValueListenableBuilder<_AttachmentSurface>(
            valueListenable: attachmentSurface,
            builder: (context, surface, _) {
              final surfaceDuration = reducedMotion
                  ? Duration.zero
                  : Duration(
                      milliseconds:
                          surface == _AttachmentSurface.camera ||
                              surface == _AttachmentSurface.photos
                          ? 320
                          : 250,
                    );
              final expandedSurfaceCoversComposer =
                  surface == _AttachmentSurface.camera ||
                  surface == _AttachmentSurface.photos;
              final overlayAnchorY =
                  composerOrigin.dy +
                  (expandedSurfaceCoversComposer
                      ? layoutInfo.childSize.height + Grid.twelve
                      : 0);
              return AnimatedPositioned(
                duration: surfaceDuration,
                curve:
                    surface == _AttachmentSurface.camera ||
                        surface == _AttachmentSurface.photos
                    ? const Cubic(0.34, 1.25, 0.64, 1)
                    : const Cubic(0.22, 1, 0.36, 1),
                left: composerOrigin.dx,
                bottom: layoutInfo.overlaySize.height - overlayAnchorY,
                width: layoutInfo.childSize.width,
                child: ClipRect(
                  child: Padding(
                    padding: const EdgeInsets.only(bottom: Grid.xxs),
                    child: surface == _AttachmentSurface.closed
                        ? _SuggestionPanelMotion(
                            duration: surfaceDuration,
                            alignment: Alignment.bottomLeft,
                            child: buildOverlayPanel(surface),
                          )
                        : buildOverlayPanel(surface),
                  ),
                ),
              );
            },
          );
        },
        child: _ComposeBarLayout(
          attachments: attachments.value,
          uploadingCount: uploadingCount.value,
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
          expansionValue: composerExpansionValue,
          expansionProgress: composerExpansionProgress,
          formattingOpen: showFormatting.value,
          onCloseFormatting: () => showFormatting.value = false,
          motionDuration: motionDuration,
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
            showEmojiPicker(context: context, onSelect: insertEmoji);
          },
          onOpenFormatting: () {
            attachmentSurface.value = _AttachmentSurface.closed;
            showFormatting.value = true;
          },
          hasPendingUploads: hasPendingUploads,
          isSending: isSending.value,
        ),
      ),
    );
  }
}
