import 'dart:collection';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
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
    final isSending = useState(false);
    final showFormatting = useState(false);
    final attachments = useState<List<BlobDescriptor>>([]);
    final uploadError = useState<String?>(null);
    final uploadingCount = useState(0);
    final clipboardHasImage = useState(false);
    final hasAttachments = attachments.value.isNotEmpty;
    final hasPendingUploads = uploadingCount.value > 0;
    final customEmoji = ref.watch(customEmojiListProvider);

    final resolvedHint =
        hintText ??
        (channelName.isNotEmpty ? 'Message #$channelName' : 'Message\u2026');

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

    void clearComposer() {
      controller.clear();
      attachments.value = [];
      mentionMap.value.clear();
      mentionQuery.value = null;
      channelQuery.value = null;
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

    // Insert an emoji at the cursor.
    void insertEmoji(String emoji) {
      final text = controller.text;
      final cursor = controller.selection.isValid
          ? controller.selection.baseOffset
          : text.length;
      final before = text.substring(0, cursor);
      final after = text.substring(cursor);
      controller.text = '$before$emoji$after';
      controller.selection = TextSelection.collapsed(
        offset: cursor + emoji.length,
      );
      focusNode.requestFocus();
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

    final hasSuggestions =
        suggestions.isNotEmpty || channelSuggestions.isNotEmpty;

    return Column(
      mainAxisSize: MainAxisSize.min,
      children: [
        // Channel suggestions (above the compose chrome).
        if (channelSuggestions.isNotEmpty)
          _ChannelSuggestions(
            suggestions: channelSuggestions,
            onSelect: insertChannel,
          ),

        // Mention suggestions (above the compose chrome).
        if (suggestions.isNotEmpty)
          _MentionSuggestions(
            suggestions: suggestions,
            userCache: userCache,
            currentPubkey: currentPubkey,
            isDmChannel: isDmChannel,
            onSelect: insertMention,
          ),

        // Compose chrome — bottom-sheet style container.
        Container(
          decoration: BoxDecoration(
            color: context.colors.surfaceContainerHighest,
            borderRadius: !hasSuggestions
                ? const BorderRadius.vertical(
                    top: Radius.circular(Radii.dialog),
                  )
                : BorderRadius.zero,
            boxShadow: !hasSuggestions
                ? [
                    BoxShadow(
                      color: context.colors.shadow.withValues(alpha: 0.08),
                      blurRadius: 8,
                      offset: const Offset(0, -2),
                    ),
                  ]
                : null,
          ),
          padding: EdgeInsets.only(
            left: Grid.gutter,
            right: Grid.gutter,
            top: Grid.xs,
            bottom: MediaQuery.viewPaddingOf(context).bottom + Grid.twelve,
          ),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              // Formatting toolbar (toggled via Aa button).
              if (showFormatting.value)
                _FormattingToolbar(onFormat: applyFormat),

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

              // Row 1 — text input (full width, grows).
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
                style: context.textTheme.bodyMedium,
                decoration: InputDecoration(
                  hintText: resolvedHint,
                  hintStyle: context.textTheme.bodyMedium?.copyWith(
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
              ),

              const SizedBox(height: Grid.xxs),

              // Row 2 — action buttons [paperclip, emoji, @, Aa] ... [send].
              Row(
                children: [
                  _ComposeAction(
                    icon: LucideIcons.paperclip,
                    onTap: () {
                      showModalBottomSheet<void>(
                        context: context,
                        showDragHandle: true,
                        builder: (sheetContext) => SafeArea(
                          child: Column(
                            mainAxisSize: MainAxisSize.min,
                            children: [
                              ListTile(
                                leading: const Icon(LucideIcons.image),
                                title: const Text('Photo'),
                                onTap: () {
                                  Navigator.of(sheetContext).pop();
                                  pickAndUpload(
                                    ref
                                        .read(mediaUploadServiceProvider)
                                        .pickAndUploadImage,
                                  );
                                },
                              ),
                              ListTile(
                                leading: const Icon(LucideIcons.video),
                                title: const Text('Video'),
                                onTap: () {
                                  Navigator.of(sheetContext).pop();
                                  pickAndUpload(
                                    ref
                                        .read(mediaUploadServiceProvider)
                                        .pickAndUploadVideo,
                                  );
                                },
                              ),
                            ],
                          ),
                        ),
                      );
                    },
                  ),
                  _ComposeAction(
                    icon: LucideIcons.smilePlus,
                    onTap: () => showEmojiPicker(
                      context: context,
                      onSelect: insertEmoji,
                    ),
                  ),
                  _ComposeAction(
                    icon: LucideIcons.atSign,
                    onTap: triggerMention,
                  ),
                  _ComposeAction(icon: LucideIcons.hash, onTap: triggerChannel),
                  _ComposeAction(
                    icon: LucideIcons.aLargeSmall,
                    active: showFormatting.value,
                    onTap: () => showFormatting.value = !showFormatting.value,
                  ),
                  const Spacer(),
                  _SendButton(
                    isDisabled: hasPendingUploads,
                    isSending: isSending.value,
                    onTap: send,
                  ),
                ],
              ),
            ],
          ),
        ),
      ],
    );
  }
}
