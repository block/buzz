import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/theme/theme.dart';
import '../../shared/widgets/frosted_app_bar.dart';
import '../../shared/widgets/frosted_scaffold.dart';
import '../channels/message_content.dart';
import '../profile/profile_provider.dart';
import '../profile/user_cache_provider.dart';
import 'note_card.dart';
import 'pulse_actions.dart';
import 'pulse_models.dart';

/// Full-screen page for composing a new note or replying to one.
///
/// When [replyTo] is null this composes a new top-level note; when set the
/// page shows the note being replied to as context and the action button
/// reads "Reply". The text field auto-focuses on open so the keyboard is
/// ready immediately.
class ComposeNotePage extends HookConsumerWidget {
  final UserNote? replyTo;

  const ComposeNotePage({super.key, this.replyTo});

  bool get _isReply => replyTo != null;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final controller = useTextEditingController();
    final focusNode = useFocusNode();
    final isSending = useState(false);
    final hasText = useListenableSelector(
      controller,
      () => controller.text.trim().isNotEmpty,
    );
    final profile = ref.watch(profileProvider).asData?.value;

    // Auto-focus the field once the page has mounted.
    useEffect(() {
      WidgetsBinding.instance.addPostFrameCallback((_) {
        focusNode.requestFocus();
      });
      return null;
    }, const []);

    Future<void> submit() async {
      if (!hasText || isSending.value) return;
      isSending.value = true;
      try {
        await publishNote(ref, content: controller.text, replyTo: replyTo);
        if (context.mounted) Navigator.of(context).pop(true);
      } catch (_) {
        // Keep the page open so the draft isn't lost; re-enable the button.
        isSending.value = false;
        rethrow;
      }
    }

    return FrostedScaffold(
      resizeToAvoidBottomInset: true,
      appBar: FrostedAppBar(
        leading: TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: const Text('Cancel'),
        ),
        actions: [
          Padding(
            padding: const EdgeInsets.only(right: Grid.xxs),
            child: FilledButton(
              onPressed: hasText && !isSending.value ? submit : null,
              style: FilledButton.styleFrom(
                padding: const EdgeInsets.symmetric(horizontal: Grid.xs),
                minimumSize: const Size(0, 36),
                shape: const StadiumBorder(),
              ),
              child: isSending.value
                  ? const SizedBox(
                      width: 16,
                      height: 16,
                      child: CircularProgressIndicator(strokeWidth: 2),
                    )
                  : Text(_isReply ? 'Reply' : 'Post'),
            ),
          ),
        ],
      ),
      body: SafeArea(
        top: false,
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            SizedBox(height: frostedAppBarHeight(context)),
            if (_isReply) _ReplyContext(note: replyTo!),
            Expanded(
              child: SingleChildScrollView(
                padding: const EdgeInsets.all(Grid.xs),
                child: Row(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    CircleAvatar(
                      radius: 18,
                      backgroundColor: context.colors.primaryContainer,
                      backgroundImage: profile?.avatarUrl != null
                          ? NetworkImage(profile!.avatarUrl!)
                          : null,
                      child: profile?.avatarUrl == null
                          ? Text(
                              profile?.initial ?? '?',
                              style: context.textTheme.labelMedium?.copyWith(
                                color: context.colors.onPrimaryContainer,
                              ),
                            )
                          : null,
                    ),
                    const SizedBox(width: Grid.xxs),
                    Expanded(
                      child: TextField(
                        controller: controller,
                        focusNode: focusNode,
                        minLines: 3,
                        maxLines: null,
                        autofocus: true,
                        keyboardType: TextInputType.multiline,
                        textInputAction: TextInputAction.newline,
                        style: context.textTheme.bodyLarge,
                        decoration: InputDecoration(
                          hintText: _isReply
                              ? 'Post your reply'
                              : 'What’s on your mind?',
                          border: InputBorder.none,
                          enabledBorder: InputBorder.none,
                          focusedBorder: InputBorder.none,
                          isDense: true,
                          contentPadding: const EdgeInsets.symmetric(
                            vertical: Grid.half,
                          ),
                        ),
                      ),
                    ),
                  ],
                ),
              ),
            ),
          ],
        ),
      ),
    );
  }
}

/// A read-only preview of the note being replied to, laid out like a list
/// row (avatar + name + time + content) but non-interactive. Capped in
/// height so a long note doesn't push the editor off-screen.
class _ReplyContext extends ConsumerWidget {
  final UserNote note;

  const _ReplyContext({required this.note});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final pubkey = note.pubkey.toLowerCase();
    final profile =
        ref.watch(userCacheProvider.select((cache) => cache[pubkey])) ??
        ref.read(userCacheProvider.notifier).get(pubkey);
    final displayName = profile?.label ?? _shortPubkey(pubkey);

    return Padding(
      padding: const EdgeInsets.fromLTRB(Grid.xs, Grid.xs, Grid.xs, 0),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(
            'Replying to $displayName',
            style: context.textTheme.labelMedium?.copyWith(
              color: context.colors.onSurfaceVariant,
            ),
          ),
          const SizedBox(height: Grid.xs),
          // Mirror the NoteCard list-row layout, read-only. Shrink-wraps to
          // the content; long notes are capped to ~3 lines via the parent
          // page scroll + ellipsis on the content.
          Row(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              CircleAvatar(
                radius: 18,
                backgroundColor: context.colors.primaryContainer,
                backgroundImage: profile?.avatarUrl != null
                    ? NetworkImage(profile!.avatarUrl!)
                    : null,
                child: profile?.avatarUrl == null
                    ? Text(
                        (profile?.initial ?? displayName[0]).toUpperCase(),
                        style: context.textTheme.labelMedium?.copyWith(
                          color: context.colors.onPrimaryContainer,
                        ),
                      )
                    : null,
              ),
              const SizedBox(width: Grid.xs),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Row(
                      children: [
                        Flexible(
                          child: Text(
                            displayName,
                            style: context.textTheme.labelMedium?.copyWith(
                              fontWeight: FontWeight.w700,
                            ),
                            overflow: TextOverflow.ellipsis,
                          ),
                        ),
                        const SizedBox(width: Grid.half),
                        Text(
                          formatPulseRelativeTime(note.createdAt),
                          style: context.textTheme.labelSmall?.copyWith(
                            color: context.colors.onSurfaceVariant,
                          ),
                        ),
                      ],
                    ),
                    const SizedBox(height: Grid.half),
                    // Rich content (images, links, mentions) like the list
                    // row, but clipped to a compact max height so a tall
                    // image or long note can't blow up the page.
                    ConstrainedBox(
                      constraints: const BoxConstraints(maxHeight: 132),
                      child: ClipRect(
                        child: Align(
                          alignment: Alignment.topLeft,
                          heightFactor: 1,
                          child: MessageContent(
                            content: note.content,
                            tags: note.tags,
                          ),
                        ),
                      ),
                    ),
                  ],
                ),
              ),
            ],
          ),
          const SizedBox(height: Grid.xs),
          Divider(
            height: 1,
            color: context.colors.outlineVariant.withValues(alpha: 0.5),
          ),
        ],
      ),
    );
  }

  String _shortPubkey(String pubkey) =>
      pubkey.length >= 8 ? '${pubkey.substring(0, 8)}...' : pubkey;
}
