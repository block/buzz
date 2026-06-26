import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import 'package:nostr/nostr.dart' as nostr;

import '../../shared/relay/relay.dart';
import '../../shared/theme/theme.dart';
import '../profile/user_cache_provider.dart';
import '../profile/user_profile.dart';
import '../custom_emoji/custom_emoji.dart';
import '../custom_emoji/custom_emoji_provider.dart';
import 'channel.dart';
import 'channel_management_provider.dart';
import 'channels_provider.dart';
import 'emoji_picker.dart';

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
    final hasAttachments = attachments.value.isNotEmpty;
    final hasPendingUploads = uploadingCount.value > 0;
    final customEmoji = ref.watch(customEmojiListProvider);

    final resolvedHint =
        hintText ??
        (channelName.isNotEmpty ? 'Message #$channelName' : 'Message\u2026');

    // Mention state --------------------------------------------------------
    final mentionQuery = useState<String?>(null);
    final mentionStartIdx = useState(-1);
    // Map of displayName → pubkey built as the user selects mentions.
    // Used to pass resolved pubkeys directly to onSend, avoiding regex.
    final mentionMap = useRef(<String, String>{});

    // Channel autocomplete state ----------------------------------------------
    final channelQuery = useState<String?>(null);
    final channelStartIdx = useState(-1);
    final channelsAsync = ref.watch(channelsProvider);

    final membersAsync = ref.watch(channelMembersProvider(channelId));
    final currentPubkey = ref.watch(currentPubkeyProvider);
    final userCache = ref.watch(userCacheProvider);

    // Preload profiles for channel members so @mention suggestions show names.
    useEffect(() {
      final memberList = membersAsync.asData?.value ?? <ChannelMember>[];
      if (memberList.isNotEmpty) {
        ref
            .read(userCacheProvider.notifier)
            .preload(memberList.map((m) => m.pubkey).toList());
      }
      return null;
    }, [membersAsync.asData?.value.length]);

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

    // Filter channel members against the query.
    final members = membersAsync.asData?.value ?? <ChannelMember>[];
    final suggestions = _filterMembers(
      members,
      mentionQuery.value,
      currentPubkey,
      userCache,
    );

    // Filter channels against the query.
    final channels = channelsAsync.asData?.value ?? <Channel>[];
    final channelSuggestions = filterChannels(channels, channelQuery.value);

    // Insert a selected mention into the text field.
    void insertMention(ChannelMember member) {
      final cached = ref.read(userCacheProvider)[member.pubkey.toLowerCase()];
      final name = cached?.displayName?.trim().isNotEmpty == true
          ? cached!.displayName!.trim()
          : '${member.pubkey.substring(0, 8)}\u2026';
      // Track the resolved pubkey so we can pass it at send time.
      mentionMap.value[name] = member.pubkey;

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
      final pubkeys = <String>[
        for (final entry in mentionMap.value.entries)
          if (text.contains('@${entry.key}')) entry.value,
      ];

      final payload = _ComposeDraftPayload.fromDraft(
        text: text,
        attachments: attachments.value,
        customEmoji: customEmoji,
      );

      isSending.value = true;
      try {
        await onSend(payload.content, pubkeys, mediaTags: payload.mediaTags);
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
            left: Grid.xs,
            right: Grid.xs,
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

const _typingThrottleMs = 3000;

/// Walk backward from [cursor] looking for [trigger] (e.g. `@` or `#`) at a
/// word boundary. Returns the index of the trigger character, or `null` if none
/// is found.
///
/// When [stopAtSpace] is `true` the walk stops at both spaces and newlines —
/// appropriate for `#channel` names which are kebab-case slugs without spaces.
/// When `false`, only newlines stop the walk, allowing multi-word queries like
/// `@Alice Smith` to match members with multi-word display names.
@visibleForTesting
int? findTrigger(
  String text,
  int cursor,
  String trigger, {
  bool stopAtSpace = true,
}) {
  for (var i = cursor - 1; i >= 0; i--) {
    final ch = text[i];
    if (ch == '\n') break;
    if (stopAtSpace && ch == ' ') break;
    if (ch == trigger) {
      if (i == 0 || text[i - 1] == ' ' || text[i - 1] == '\n') {
        return i;
      }
      break;
    }
  }
  return null;
}

/// Replace the range `[start, cursor)` with [replacement] and move the cursor
/// to the end of the replacement. Used by both mention and channel insertion.
@visibleForTesting
void spliceAndMoveCursor(
  TextEditingController controller,
  FocusNode focusNode, {
  required int start,
  required String replacement,
}) {
  final text = controller.text;
  final cursor =
      (controller.selection.isValid
              ? controller.selection.baseOffset
              : text.length)
          .clamp(start, text.length);

  final before = text.substring(0, start);
  final after = text.substring(cursor);
  controller.text = '$before$replacement$after';
  controller.selection = TextSelection.collapsed(
    offset: start + replacement.length,
  );
  focusNode.requestFocus();
}

/// Insert [trigger] (e.g. `@` or `#`) at the cursor position, prefixed with
/// a space if needed for word separation. Used by `triggerMention` and
/// `triggerChannel`.
void _insertTriggerAtCursor(
  TextEditingController controller,
  FocusNode focusNode,
  String trigger,
) {
  final text = controller.text;
  final cursor = controller.selection.isValid
      ? controller.selection.baseOffset
      : text.length;
  final needsSpace =
      cursor > 0 && text[cursor - 1] != ' ' && text[cursor - 1] != '\n';
  final insert = needsSpace ? ' $trigger' : trigger;
  final before = text.substring(0, cursor);
  final after = text.substring(cursor);
  controller.text = '$before$insert$after';
  controller.selection = TextSelection.collapsed(
    offset: cursor + insert.length,
  );
  focusNode.requestFocus();
}

/// Send a typing indicator over the WebSocket (fire-and-forget).
///
/// Desktop sends these as `["EVENT", signedEvent]` over the WebSocket — not
/// via HTTP. Ephemeral events like typing indicators are broadcast-only and
/// the relay doesn't persist them, so the HTTP `/api/events` endpoint may
/// silently discard them.
void _sendTypingIndicator(
  WidgetRef ref, {
  required String channelId,
  String? threadHeadId,
  String? rootId,
}) {
  try {
    final config = ref.read(relayConfigProvider);
    final nsec = config.nsec;
    if (nsec == null || nsec.isEmpty) return;

    final privkeyHex = nostr.Nip19.decode(payload: nsec).data;
    if (privkeyHex.isEmpty) return;

    final tags = <List<String>>[
      ['h', channelId],
      if (threadHeadId != null && rootId != null && rootId != threadHeadId) ...[
        ['e', rootId, '', 'root'],
        ['e', threadHeadId, '', 'reply'],
      ] else if (threadHeadId != null)
        ['e', threadHeadId, '', 'reply'],
    ];

    final event = nostr.Event.from(
      kind: EventKind.typingIndicator,
      content: '',
      tags: tags,
      secretKey: privkeyHex,
      verify: false,
    );

    // Send directly over WebSocket — fire-and-forget, matching desktop.
    final session = ref.read(relaySessionProvider.notifier);
    session.sendRaw(['EVENT', event.toMap()]);
  } catch (_) {
    // Fire-and-forget — typing indicator failure is non-fatal.
  }
}

List<ChannelMember> _filterMembers(
  List<ChannelMember> members,
  String? query,
  String? currentPubkey,
  Map<String, UserProfile> userCache,
) {
  if (query == null) return const [];
  final q = query.toLowerCase();
  return members
      .where(
        (m) =>
            currentPubkey == null ||
            m.pubkey.toLowerCase() != currentPubkey.toLowerCase(),
      )
      .where((m) {
        if (q.isEmpty) return true;
        final profile = userCache[m.pubkey.toLowerCase()];
        final name = (profile?.displayName ?? m.displayName ?? '')
            .toLowerCase();
        final firstName = name.split(RegExp(r'\s+')).first;
        return name.startsWith(q) ||
            firstName.startsWith(q) ||
            name.contains(q);
      })
      .take(6)
      .toList();
}

class _MentionSuggestions extends StatelessWidget {
  final List<ChannelMember> suggestions;
  final Map<String, UserProfile> userCache;
  final String? currentPubkey;
  final void Function(ChannelMember) onSelect;

  const _MentionSuggestions({
    required this.suggestions,
    required this.userCache,
    required this.currentPubkey,
    required this.onSelect,
  });

  @override
  Widget build(BuildContext context) {
    return Container(
      constraints: const BoxConstraints(maxHeight: 240),
      clipBehavior: Clip.hardEdge,
      decoration: BoxDecoration(
        color: context.colors.surfaceContainerHighest,
        borderRadius: const BorderRadius.vertical(
          top: Radius.circular(Radii.dialog),
        ),
        boxShadow: [
          BoxShadow(
            color: context.colors.shadow.withValues(alpha: 0.08),
            blurRadius: 8,
            offset: const Offset(0, -2),
          ),
        ],
      ),
      child: ListView.separated(
        shrinkWrap: true,
        padding: const EdgeInsets.symmetric(vertical: Grid.xxs),
        itemCount: suggestions.length,
        separatorBuilder: (_, _) => const SizedBox.shrink(),
        itemBuilder: (context, index) {
          final member = suggestions[index];
          final profile = userCache[member.pubkey.toLowerCase()];
          final name = profile?.displayName?.trim().isNotEmpty == true
              ? profile!.displayName!.trim()
              : member.labelFor(currentPubkey);
          final avatarUrl = profile?.avatarUrl;
          final initial =
              (profile?.displayName?.trim().isNotEmpty == true
                      ? profile!.displayName!.trim()
                      : member.pubkey)[0]
                  .toUpperCase();

          return ListTile(
            dense: true,
            visualDensity: VisualDensity.compact,
            leading: CircleAvatar(
              radius: 14,
              backgroundColor: context.colors.primaryContainer,
              backgroundImage: avatarUrl != null
                  ? NetworkImage(avatarUrl)
                  : null,
              child: avatarUrl == null
                  ? Text(
                      initial,
                      style: context.textTheme.labelSmall?.copyWith(
                        color: context.colors.onPrimaryContainer,
                      ),
                    )
                  : null,
            ),
            title: Text(name, style: context.textTheme.bodyMedium),
            trailing: member.isBot
                ? Icon(
                    LucideIcons.bot,
                    size: 14,
                    color: context.colors.onSurfaceVariant,
                  )
                : null,
            onTap: () => onSelect(member),
          );
        },
      ),
    );
  }
}

@visibleForTesting
List<Channel> filterChannels(List<Channel> channels, String? query) {
  if (query == null) return const [];
  final q = query.toLowerCase();
  return channels
      .where((c) => c.channelType != 'dm')
      .where((c) {
        if (q.isEmpty) return true;
        return c.name.toLowerCase().contains(q);
      })
      .take(8)
      .toList();
}

class _ChannelSuggestions extends StatelessWidget {
  final List<Channel> suggestions;
  final void Function(Channel) onSelect;

  const _ChannelSuggestions({
    required this.suggestions,
    required this.onSelect,
  });

  @override
  Widget build(BuildContext context) {
    return Container(
      constraints: const BoxConstraints(maxHeight: 240),
      clipBehavior: Clip.hardEdge,
      decoration: BoxDecoration(
        color: context.colors.surfaceContainerHighest,
        borderRadius: const BorderRadius.vertical(
          top: Radius.circular(Radii.dialog),
        ),
        boxShadow: [
          BoxShadow(
            color: context.colors.shadow.withValues(alpha: 0.08),
            blurRadius: 8,
            offset: const Offset(0, -2),
          ),
        ],
      ),
      child: ListView.separated(
        shrinkWrap: true,
        padding: const EdgeInsets.symmetric(vertical: Grid.xxs),
        itemCount: suggestions.length,
        separatorBuilder: (_, _) => const SizedBox.shrink(),
        itemBuilder: (context, index) {
          final channel = suggestions[index];
          return ListTile(
            dense: true,
            visualDensity: VisualDensity.compact,
            leading: Icon(
              channel.isForum ? LucideIcons.messageSquare : LucideIcons.hash,
              size: 18,
              color: context.colors.onSurfaceVariant,
            ),
            title: Text(
              '#${channel.name}',
              style: context.textTheme.bodyMedium,
            ),
            trailing: Text(
              channel.channelType,
              style: context.textTheme.labelSmall?.copyWith(
                color: context.colors.onSurfaceVariant,
              ),
            ),
            onTap: () => onSelect(channel),
          );
        },
      ),
    );
  }
}

class _FormattingToolbar extends StatelessWidget {
  final void Function(String prefix, [String? suffix]) onFormat;

  const _FormattingToolbar({required this.onFormat});

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.only(bottom: Grid.half),
      child: Row(
        children: [
          _FormatButton(
            icon: LucideIcons.bold,
            tooltip: 'Bold',
            onTap: () => onFormat('**'),
          ),
          _FormatButton(
            icon: LucideIcons.italic,
            tooltip: 'Italic',
            onTap: () => onFormat('_'),
          ),
          _FormatButton(
            icon: LucideIcons.strikethrough,
            tooltip: 'Strikethrough',
            onTap: () => onFormat('~~'),
          ),
          _FormatButton(
            icon: LucideIcons.code,
            tooltip: 'Code',
            onTap: () => onFormat('`'),
          ),
          _FormatButton(
            icon: LucideIcons.squareCode,
            tooltip: 'Code block',
            onTap: () => onFormat('```\n', '\n```'),
          ),
        ],
      ),
    );
  }
}

class _FormatButton extends StatelessWidget {
  final IconData icon;
  final String tooltip;
  final VoidCallback onTap;

  const _FormatButton({
    required this.icon,
    required this.tooltip,
    required this.onTap,
  });

  @override
  Widget build(BuildContext context) {
    return Tooltip(
      message: tooltip,
      child: InkWell(
        borderRadius: BorderRadius.circular(Radii.sm),
        onTap: onTap,
        child: Padding(
          padding: const EdgeInsets.all(Grid.xxs),
          child: Icon(icon, size: 18, color: context.colors.onSurfaceVariant),
        ),
      ),
    );
  }
}

class _ComposeAction extends StatelessWidget {
  final IconData icon;
  final bool active;
  final VoidCallback onTap;

  const _ComposeAction({
    required this.icon,
    this.active = false,
    required this.onTap,
  });

  @override
  Widget build(BuildContext context) {
    return SizedBox(
      width: 36,
      height: 36,
      child: IconButton(
        onPressed: onTap,
        icon: Icon(
          icon,
          size: 20,
          color: active
              ? context.colors.primary
              : context.colors.onSurfaceVariant,
        ),
        padding: EdgeInsets.zero,
        visualDensity: VisualDensity.compact,
      ),
    );
  }
}

@immutable
class _ComposeDraftPayload {
  final String content;
  final List<List<String>> mediaTags;

  const _ComposeDraftPayload({required this.content, required this.mediaTags});

  factory _ComposeDraftPayload.fromDraft({
    required String text,
    required List<BlobDescriptor> attachments,
    required List<CustomEmoji> customEmoji,
  }) {
    var content = text;
    final mediaTags = <List<String>>[];
    for (final attachment in attachments) {
      mediaTags.add(attachment.toImetaTag());
      content += '\n${attachment.toMarkdownImage()}';
    }
    mediaTags.addAll(buildCustomEmojiTags(content, customEmoji));
    return _ComposeDraftPayload(content: content, mediaTags: mediaTags);
  }
}

List<BlobDescriptor> _withoutAttachment(
  List<BlobDescriptor> attachments,
  String url,
) {
  return [
    for (final attachment in attachments)
      if (attachment.url != url) attachment,
  ];
}

class _AttachmentStrip extends StatelessWidget {
  final List<BlobDescriptor> attachments;
  final int uploadingCount;
  final void Function(String url) onRemove;

  const _AttachmentStrip({
    required this.attachments,
    required this.uploadingCount,
    required this.onRemove,
  });

  @override
  Widget build(BuildContext context) {
    final thumbWidth = 72.0;
    final thumbHeight = 72.0;

    return SizedBox(
      height: thumbHeight,
      child: ListView.separated(
        scrollDirection: Axis.horizontal,
        itemCount: attachments.length + uploadingCount,
        separatorBuilder: (_, _) => const SizedBox(width: Grid.half),
        itemBuilder: (context, index) {
          if (index >= attachments.length) {
            return Container(
              width: thumbWidth,
              decoration: BoxDecoration(
                color: context.colors.surface,
                borderRadius: BorderRadius.circular(Radii.md),
                border: Border.all(color: context.colors.outlineVariant),
              ),
              child: const Center(
                child: CircularProgressIndicator(strokeWidth: 2),
              ),
            );
          }

          final attachment = attachments[index];
          final isVideo = attachment.type.startsWith('video/');
          final previewUrl = attachment.thumb ?? attachment.url;
          return Container(
            key: ValueKey('compose-attachment:${attachment.url}'),
            width: thumbWidth,
            decoration: BoxDecoration(
              borderRadius: BorderRadius.circular(Radii.md),
              border: Border.all(color: context.colors.outlineVariant),
            ),
            child: Stack(
              fit: StackFit.expand,
              children: [
                ClipRRect(
                  borderRadius: BorderRadius.circular(Radii.md),
                  child: isVideo
                      ? ColoredBox(
                          color: Colors.black,
                          child: Center(
                            child: Icon(
                              LucideIcons.video,
                              color: Colors.white,
                              size: 24,
                            ),
                          ),
                        )
                      : Image.network(
                          previewUrl,
                          fit: BoxFit.cover,
                          errorBuilder: (_, _, _) => ColoredBox(
                            color: context.colors.surface,
                            child: Icon(
                              LucideIcons.image,
                              color: context.colors.onSurfaceVariant,
                            ),
                          ),
                        ),
                ),
                Positioned(
                  top: Grid.quarter,
                  right: Grid.quarter,
                  child: SizedBox(
                    width: 24,
                    height: 24,
                    child: IconButton(
                      onPressed: () => onRemove(attachment.url),
                      tooltip: 'Remove attachment',
                      visualDensity: VisualDensity.compact,
                      style: IconButton.styleFrom(
                        backgroundColor: context.colors.surface.withValues(
                          alpha: 0.92,
                        ),
                        minimumSize: const Size(24, 24),
                        maximumSize: const Size(24, 24),
                        padding: EdgeInsets.zero,
                        tapTargetSize: MaterialTapTargetSize.shrinkWrap,
                      ),
                      icon: Icon(
                        LucideIcons.x,
                        size: 14,
                        color: context.colors.onSurface,
                      ),
                    ),
                  ),
                ),
              ],
            ),
          );
        },
      ),
    );
  }
}

class _SendButton extends StatelessWidget {
  final bool isSending;
  final bool isDisabled;
  final VoidCallback onTap;

  const _SendButton({
    required this.isSending,
    required this.onTap,
    this.isDisabled = false,
  });

  @override
  Widget build(BuildContext context) {
    return SizedBox(
      width: 36,
      height: 36,
      child: IconButton(
        onPressed: (isSending || isDisabled) ? null : onTap,
        style: IconButton.styleFrom(
          backgroundColor: context.colors.primary,
          disabledBackgroundColor: context.colors.primary.withValues(
            alpha: 0.5,
          ),
          shape: RoundedRectangleBorder(
            borderRadius: BorderRadius.circular(Radii.md),
          ),
        ),
        padding: EdgeInsets.zero,
        icon: isSending
            ? SizedBox(
                width: 16,
                height: 16,
                child: CircularProgressIndicator(
                  strokeWidth: 2,
                  color: context.colors.onPrimary,
                ),
              )
            : Icon(
                LucideIcons.sendHorizontal,
                size: 18,
                color: context.colors.onPrimary,
              ),
      ),
    );
  }
}

String _formatUploadError(Object error) {
  return error.toString().replaceFirst('Exception: ', '');
}
