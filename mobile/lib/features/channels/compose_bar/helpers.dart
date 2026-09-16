part of '../compose_bar.dart';

String _composerDraftIdentity(WidgetRef ref) =>
    '${ref.watch(relayConfigProvider).baseUrl}'
    ':${ref.watch(myPubkeyProvider) ?? 'anon'}';

void _useComposerFocusRestorer({
  required ValueChanged<VoidCallback>? onChanged,
  required ValueNotifier<bool> isExpanded,
  required FocusNode focusNode,
  required VoidCallback expand,
}) {
  useEffect(() {
    if (onChanged == null) return null;

    var isCurrent = true;
    void restoreFocus() {
      if (!isCurrent) return;
      if (isExpanded.value) {
        focusNode.requestFocus();
      } else {
        expand();
      }
    }

    onChanged(restoreFocus);
    return () => isCurrent = false;
  }, [onChanged, focusNode]);
}

void _useComposerChannelNames(
  _MarkdownEditingController controller,
  AsyncValue<List<Channel>> channelsAsync,
) {
  final channelNames = {
    for (final channel in channelsAsync.asData?.value ?? const <Channel>[])
      channel.name.toLowerCase(): channel.id,
  };
  final channelNamesKey = channelNames.entries
      .map((entry) => '${entry.key}\u0000${entry.value}')
      .join('\u0001');
  useEffect(() {
    controller.setChannelNames(channelNames);
    return null;
  }, [controller, channelNamesKey]);
}

const _typingThrottleMs = 3000;

class _ComposerKeyboardMetricsObserver with WidgetsBindingObserver {
  final FlutterView view;
  final VoidCallback onKeyboardShown;
  final VoidCallback onKeyboardHidden;
  bool _wasVisible;

  _ComposerKeyboardMetricsObserver({
    required this.view,
    required this.onKeyboardShown,
    required this.onKeyboardHidden,
  }) : _wasVisible = view.viewInsets.bottom > 0;

  @override
  void didChangeMetrics() {
    final isVisible = view.viewInsets.bottom > 0;
    if (!_wasVisible && isVisible) onKeyboardShown();
    if (_wasVisible && !isVisible) onKeyboardHidden();
    _wasVisible = isVisible;
  }
}

void _runComposerAction(VoidCallback action) {
  unawaited(HapticFeedback.selectionClick());
  action();
}

void _showComposerEmojiPicker(
  BuildContext context,
  ValueChanged<String> onSelect,
  VoidCallback onDismiss,
) {
  showEmojiPicker(
    context: context,
    onSelect: (emoji) => _runComposerAction(() => onSelect(emoji)),
    onDismiss: onDismiss,
  );
}

void _dismissComposerKeyboard(FocusNode focusNode) {
  focusNode.unfocus();
  unawaited(SystemChannels.textInput.invokeMethod<void>('TextInput.hide'));
}

void _chooseComposerAttachment(
  BuildContext context,
  ValueNotifier<_AttachmentSurface> attachmentSurface,
  ValueNotifier<String?> uploadError,
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

Duration _composerMotionDuration(
  bool reducedMotion,
  _AttachmentSurface surface,
) => reducedMotion
    ? Duration.zero
    : Duration(
        milliseconds:
            surface == _AttachmentSurface.camera ||
                surface == _AttachmentSurface.photos
            ? 320
            : 250,
      );

void _expandComposer({
  required BuildContext context,
  required ValueNotifier<bool> isExpanded,
  required ValueNotifier<_AttachmentSurface> attachmentSurface,
  required VoidCallback? onFocusRequested,
  required FocusNode focusNode,
  required FlutterView view,
  required ValueNotifier<bool> androidImeTransitionStarted,
  required ObjectRef<Timer?> androidImeFallbackTimer,
}) {
  if (isExpanded.value) return;
  attachmentSurface.value = _AttachmentSurface.closed;
  onFocusRequested?.call();
  isExpanded.value = true;
  // Attach the editor before requesting focus so native restoration cannot
  // reopen a composer behind a popped route.
  WidgetsBinding.instance.addPostFrameCallback((_) {
    if (context.mounted && isExpanded.value) focusNode.requestFocus();
  });
  if (defaultTargetPlatform != TargetPlatform.android) return;
  androidImeFallbackTimer.value?.cancel();
  if (view.viewInsets.bottom > 0) {
    androidImeTransitionStarted.value = true;
    return;
  }
  androidImeTransitionStarted.value = false;
  androidImeFallbackTimer.value = Timer(const Duration(milliseconds: 250), () {
    if (context.mounted && isExpanded.value) {
      androidImeTransitionStarted.value = true;
    }
  });
}

Widget _composerSuggestionPanel({
  required List<Channel> channelSuggestions,
  required List<MentionCandidate> mentionSuggestions,
  required Map<String, UserProfile> userCache,
  required String? currentPubkey,
  required bool isDmChannel,
  required ValueChanged<Channel> onChannelSelect,
  required ValueChanged<MentionCandidate> onMentionSelect,
}) => channelSuggestions.isNotEmpty
    ? KeyedSubtree(
        key: const ValueKey('channel-suggestions'),
        child: _ChannelSuggestions(
          suggestions: channelSuggestions,
          onSelect: onChannelSelect,
        ),
      )
    : mentionSuggestions.isNotEmpty
    ? KeyedSubtree(
        key: const ValueKey('mention-suggestions'),
        child: _MentionSuggestions(
          suggestions: mentionSuggestions,
          userCache: userCache,
          currentPubkey: currentPubkey,
          isDmChannel: isDmChannel,
          onSelect: onMentionSelect,
        ),
      )
    : const SizedBox.shrink(key: ValueKey('no-suggestions'));

Widget _composerAttachmentPanel({
  required _AttachmentSurface surface,
  required Widget suggestionPanel,
  required VoidCallback onBack,
  required VoidCallback onCamera,
  required VoidCallback onPhotos,
  required VoidCallback onVideo,
  required VoidCallback onVoiceNote,
  required VoidCallback onFiles,
  required Future<void> Function(XFile image) onCapture,
  required Future<List<XFile>> Function() onPickAllPhotos,
  required Future<void> Function(List<XFile> photos) onChoosePhotos,
  required Future<void> Function(List<XFile> photos) onChooseAllPhotos,
}) => _AttachmentSurfacePanel(
  key: ValueKey(
    surface == _AttachmentSurface.closed
        ? 'composer-suggestions'
        : 'attachment-surface',
  ),
  surface: surface,
  suggestionPanel: suggestionPanel,
  onBack: onBack,
  onCamera: onCamera,
  onPhotos: onPhotos,
  onVideo: onVideo,
  onVoiceNote: onVoiceNote,
  onFiles: onFiles,
  onCapture: onCapture,
  onPickAllPhotos: onPickAllPhotos,
  onChoosePhotos: onChoosePhotos,
  onChooseAllPhotos: onChooseAllPhotos,
);

const _pastedImageMimeTypes = <String>[
  'image/jpeg',
  'image/jpg',
  'image/png',
  'image/webp',
];

/// Cap on ranked mention suggestions shown — matches desktop's
/// `MENTION_SUGGESTION_LIMIT`.
const _mentionSuggestionLimit = 50;

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
  controller.value = TextEditingValue(
    text: '$before$replacement$after',
    selection: TextSelection.collapsed(offset: start + replacement.length),
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
  controller.value = TextEditingValue(
    text: '$before$insert$after',
    selection: TextSelection.collapsed(offset: cursor + insert.length),
  );
  focusNode.requestFocus();
}

bool hasMention(String text, String name) {
  final pattern = RegExp(
    '(?:^|\\s|[*_]{1,3}|\\|\\|)@${RegExp.escape(name)}(?=\\|\\||[\\s,;.!?:)\\]}*_]|\$)',
    caseSensitive: false,
  );
  return pattern.hasMatch(text);
}

/// Outcome of the non-member mention prompt. `null` (dialog dismissed)
/// cancels the send and keeps the draft.
enum _NonMemberMentionChoice { invite, sendWithoutInviting }

/// User-facing text for a failed add or send.
String _composeSendErrorMessage(Object error) {
  if (error is AddMembersException) return error.message;
  return error.toString().replaceFirst('Exception: ', '');
}

/// Ask whether to invite mentioned humans who aren't channel members, or
/// send without inviting them. Mirrors desktop's `NonMemberMentionDialog`.
/// [canInvite] false (a private channel the sender doesn't own/administer)
/// drops the Invite action — the relay rejects that add, so offering it would
/// only produce an error.
Future<_NonMemberMentionChoice?> _promptNonMemberMention(
  BuildContext context, {
  required List<String> names,
  required bool canInvite,
}) {
  final verb = names.length == 1 ? 'is' : 'are';
  return showBuzzDialog<_NonMemberMentionChoice>(
    context: context,
    builder: (dialogContext) => AlertDialog(
      title: const Text('Mention people outside this channel?'),
      content: Text(
        canInvite
            ? '${names.join(', ')} $verb not in this channel. Invite them to '
                  'the channel, or send without inviting them.'
            : '${names.join(', ')} $verb not in this channel. '
                  '$privateChannelAddDeniedMessage You can still send without '
                  'inviting them.',
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(
            dialogContext,
          ).pop(_NonMemberMentionChoice.sendWithoutInviting),
          child: Text(canInvite ? 'Do nothing' : 'Send anyway'),
        ),
        if (canInvite)
          TextButton(
            onPressed: () =>
                Navigator.of(dialogContext).pop(_NonMemberMentionChoice.invite),
            child: const Text('Invite'),
          ),
      ],
    ),
  );
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

/// Reports a send that was cancelled because the active community changed.
///
/// The send path is fire-and-forget, so a `StateError` escaping it would be
/// silent. The composer's own error line cannot carry this message either: the
/// identity change that causes the failure also resets that state on the next
/// frame, so [messenger] must be resolved before the send's first `await`.
void _reportSendCancelledByCommunitySwitch(ScaffoldMessengerState? messenger) {
  messenger?.showSnackBar(
    const SnackBar(content: Text('Message not sent: the community changed')),
  );
}

/// What an add attempt left undone: who is still a non-member, and why.
@immutable
class _NonMemberAddOutcome {
  final List<String> notAdded;
  final List<String> errors;

  const _NonMemberAddOutcome({required this.notAdded, required this.errors});

  static const empty = _NonMemberAddOutcome(notAdded: [], errors: []);
}

/// Adds mentioned non-members to the channel before a send.
///
/// Agents are added silently with the `bot` role; humans are only passed here
/// after they have been explicitly invited from the mention prompt.
///
/// A rejection is reported, never thrown: the send is fire-and-forget, so an
/// escaping error would drop the message with nothing shown. [StateError] still
/// propagates — a community switch must cancel the whole send.
Future<_NonMemberAddOutcome> _addMentionedNonMembers(
  ChannelActions channelActions, {
  required String channelId,
  required List<String> agentPubkeys,
  required List<String> humanPubkeys,
  required bool canAddMembers,
}) async {
  final pending = [
    if (agentPubkeys.isNotEmpty) (agentPubkeys, 'bot'),
    if (humanPubkeys.isNotEmpty) (humanPubkeys, 'member'),
  ];
  if (pending.isEmpty) return _NonMemberAddOutcome.empty;

  // A non-member cannot add anyone to a private channel: skip the doomed
  // kind:9000 rather than trading it for a relay rejection.
  if (!canAddMembers) {
    return _NonMemberAddOutcome(
      notAdded: [for (final (pubkeys, _) in pending) ...pubkeys],
      errors: const [privateChannelAddDeniedMessage],
    );
  }

  final notAdded = <String>[];
  final errors = <String>[];
  for (final (pubkeys, role) in pending) {
    try {
      await channelActions.addMembers(
        channelId: channelId,
        pubkeys: pubkeys,
        role: role,
      );
    } on StateError {
      rethrow;
    } catch (error) {
      notAdded.addAll(
        error is AddMembersException ? error.failures.keys : pubkeys,
      );
      errors.add(_composeSendErrorMessage(error));
    }
  }
  return _NonMemberAddOutcome(notAdded: notAdded, errors: errors);
}

/// Mentioned identities that aren't in the channel yet, plus whether the sender
/// is allowed to add them at all.
@immutable
class _NonMemberMentionScan {
  final String channelId;
  final List<String> agentPubkeys;
  final List<MentionCandidate> humans;
  final bool canAddMembers;

  const _NonMemberMentionScan({
    required this.channelId,
    required this.agentPubkeys,
    required this.humans,
    required this.canAddMembers,
  });
}

/// Resolves which mentioned identities are non-members, and whether this
/// identity may add them (see [Channel.canAddMembers]). DMs are skipped: their
/// participant set is fixed at creation.
Future<_NonMemberMentionScan> _scanNonMemberMentions(
  WidgetRef ref, {
  required String channelId,
  required List<MentionCandidate> selectedMentions,
  required String? currentPubkey,
}) async {
  final none = _NonMemberMentionScan(
    channelId: channelId,
    agentPubkeys: const [],
    humans: const [],
    canAddMembers: true,
  );
  if (selectedMentions.isEmpty) return none;

  final channel = (await ref.read(
    channelsProvider.future,
  )).firstWhere((candidate) => candidate.id == channelId);
  if (channel.isDm) return none;

  final members = await ref.read(channelMembersProvider(channelId).future);
  final memberPubkeys = {
    for (final member in members) member.pubkey.toLowerCase(),
  };
  String? selfRole;
  if (currentPubkey != null) {
    final self = currentPubkey.toLowerCase();
    for (final member in members) {
      if (member.pubkey.toLowerCase() == self) {
        selfRole = member.role;
        break;
      }
    }
  }

  final agentPubkeys = <String>[];
  final humans = <MentionCandidate>[];
  final seen = <String>{};
  for (final candidate in selectedMentions) {
    final pubkey = candidate.pubkey.toLowerCase();
    if (memberPubkeys.contains(pubkey) || !seen.add(pubkey)) continue;
    if (candidate.isAgent) {
      agentPubkeys.add(pubkey);
    } else {
      humans.add(candidate);
    }
  }

  return _NonMemberMentionScan(
    channelId: channelId,
    agentPubkeys: agentPubkeys,
    humans: humans,
    canAddMembers: channel.canAddMembers(selfRole),
  );
}

/// The p-tags and `mention` reference tags an outgoing message should carry.
///
/// Anyone who ends up *not* added is demoted from a p-tag to a reference tag so
/// their name still renders without notifying a non-member — mirrors desktop's
/// `mergeOutgoingTagsWithReferenceMentions`.
class _OutgoingMentions {
  List<String> pubkeys;
  final List<List<String>> referenceTags = [];
  List<String> _invitedHumanPubkeys = const [];

  _OutgoingMentions(List<MentionCandidate> selectedMentions)
    : pubkeys = LinkedHashSet<String>.from(
        selectedMentions.map((candidate) => candidate.pubkey.toLowerCase()),
      ).toList();

  void demote(Iterable<String> demoted) {
    final excluded = {for (final pubkey in demoted) pubkey.toLowerCase()};
    if (excluded.isEmpty) return;
    pubkeys = [
      for (final pubkey in pubkeys)
        if (!excluded.contains(pubkey)) pubkey,
    ];
    referenceTags.addAll([
      for (final pubkey in excluded) ['mention', pubkey],
    ]);
  }

  /// Applies the mention prompt's outcome: invite them, or send without.
  void resolveHumanChoice(
    _NonMemberMentionChoice choice,
    List<MentionCandidate> humans,
  ) {
    final humanPubkeys = [
      for (final candidate in humans) candidate.pubkey.toLowerCase(),
    ];
    switch (choice) {
      case _NonMemberMentionChoice.invite:
        _invitedHumanPubkeys = humanPubkeys;
      case _NonMemberMentionChoice.sendWithoutInviting:
        demote(humanPubkeys);
    }
  }

  /// Adds the scanned non-members, demoting and reporting whatever didn't land.
  Future<void> addNonMembers(
    ChannelActions channelActions, {
    required _NonMemberMentionScan scan,
    required ScaffoldMessengerState? messenger,
  }) async {
    final outcome = await _addMentionedNonMembers(
      channelActions,
      channelId: scan.channelId,
      agentPubkeys: scan.agentPubkeys,
      humanPubkeys: _invitedHumanPubkeys,
      canAddMembers: scan.canAddMembers,
    );
    demote(outcome.notAdded);
    if (outcome.errors.isNotEmpty) {
      messenger?.showSnackBar(
        SnackBar(content: Text(outcome.errors.join(' '))),
      );
    }
  }
}
