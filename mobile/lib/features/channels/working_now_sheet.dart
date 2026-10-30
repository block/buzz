import 'package:buzz/shared/theme/buzz_icons.dart';
import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/profile/user_cache_provider.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/modal_presentation.dart';
import 'agent_activity/agent_activity_sheet.dart';
import 'channel_identity_names_provider.dart';
import 'conversation_activity_state.dart';
import 'small_avatar.dart';
import 'timeline_message.dart';

/// Opens desktop's "Working now" list for [scope].
Future<void> showWorkingNowSheet(
  BuildContext context, {
  required ActivityScope scope,
  List<TimelineMessage> messages = const [],
  void Function(String rootId)? onOpenThread,
}) => showBuzzModalBottomSheet<void>(
  context: context,
  title: 'Working now',
  showDragHandle: true,
  builder: (_) => WorkingNowSheet(
    scope: scope,
    messages: messages,
    onOpenThread: onOpenThread,
  ),
);

/// Opens what [pubkey] has been doing in [channelId].
Future<void> showAgentActivity(
  BuildContext context, {
  required String channelId,
  required String pubkey,
}) => showBuzzModalBottomSheet<void>(
  context: context,
  isScrollControlled: true,
  showDragHandle: true,
  builder: (_) => AgentActivitySheet(channelId: channelId, agentPubkey: pubkey),
);

/// The user's agents working in [scope], where each works, and where each
/// row leads: its thread, or its activity.
class WorkingNowSheet extends HookConsumerWidget {
  const WorkingNowSheet({
    super.key,
    required this.scope,
    this.messages = const [],
    this.onOpenThread,
  });

  final ActivityScope scope;

  /// Loaded messages, to name the threads agents work in.
  final List<TimelineMessage> messages;

  /// Opens a thread by its root, loaded or not. Null where threads do not
  /// open, as in Me.
  final void Function(String rootId)? onOpenThread;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final activity = watchConversationActivity(ref, scope);
    final agents = activity?.agents ?? const {};
    final idle = activity != null && agents.isEmpty;
    // Desktop's list closes with its indicator when the last agent stops.
    useEffect(() {
      if (!idle) return null;
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (context.mounted && (ModalRoute.of(context)?.isCurrent ?? false)) {
          Navigator.of(context).pop();
        }
      });
      return null;
    }, [idle]);
    final userCache = ref.watch(userCacheProvider);
    final labels = watchChannelIdentityLabels(
      ref,
      scope.channelId,
      agents.keys,
    );
    final navigator = Navigator.of(context);

    // Leave the sheet before the next page or sheet opens, once: a second
    // tap in the same frame must not pop the page under it.
    void then(VoidCallback action) {
      if (!(ModalRoute.of(context)?.isCurrent ?? false)) return;
      navigator.pop();
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (navigator.mounted) action();
      });
    }

    void viewActivity(String pubkey) => then(
      () => showAgentActivity(
        navigator.context,
        channelId: scope.channelId,
        pubkey: pubkey,
      ),
    );

    TimelineMessage? message(String id) => messages
        .where((message) => message.id.toLowerCase() == id.toLowerCase())
        .firstOrNull;

    // Desktop's scope label: up to two places, then a count.
    String? place(List<String?> roots) {
      if (scope.threadRootId != null) return null;
      return [
        for (final root in roots.take(2))
          if (root == null)
            'Channel conversation'
          else
            'Thread · ${_preview(message(root)) ?? root.substring(0, 8)}',
        if (roots.length > 2) '+${roots.length - 2} more',
      ].join(' / ');
    }

    return SafeArea(
      top: false,
      // The sheet's height is capped, so a long list scrolls.
      child: SingleChildScrollView(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            for (final MapEntry(key: pubkey, value: roots) in agents.entries)
              Builder(
                builder: (context) {
                  final name = labels[pubkey] ?? pubkey.substring(0, 10);
                  final known = roots.whereType<String>().toList();
                  final root =
                      scope.threadRootId ??
                      (known.length == 1 ? known.single : null);
                  // Opening the conversation already shown would do nothing,
                  // so, as on desktop, the row opens the agent's activity.
                  final opens = onOpenThread;
                  final here = root == scope.threadRootId || opens == null;
                  final open = here
                      ? () => viewActivity(pubkey)
                      : () => then(() => opens(root!));
                  final where = place(roots);
                  return Row(
                    key: ValueKey('working-now-$pubkey'),
                    children: [
                      Expanded(
                        child: Semantics(
                          button: true,
                          label: here
                              ? 'View $name activity'
                              : 'Open thread for $name',
                          onTap: open,
                          excludeSemantics: true,
                          child: InkWell(
                            onTap: open,
                            child: Padding(
                              padding: const EdgeInsets.symmetric(
                                horizontal: Grid.gutter,
                                vertical: Grid.xxs,
                              ),
                              child: Row(
                                children: [
                                  SmallAvatar(
                                    pubkey: pubkey,
                                    userCache: userCache,
                                    size: Grid.lg,
                                    agent: true,
                                  ),
                                  const SizedBox(width: Grid.twelve),
                                  Expanded(
                                    child: Column(
                                      crossAxisAlignment:
                                          CrossAxisAlignment.start,
                                      children: [
                                        Text(
                                          name,
                                          maxLines: 1,
                                          overflow: TextOverflow.ellipsis,
                                          style: context.textTheme.titleSmall,
                                        ),
                                        Text(
                                          'Working',
                                          style: context.textTheme.bodySmall
                                              ?.copyWith(
                                                color: context
                                                    .colors
                                                    .onSurfaceVariant,
                                              ),
                                        ),
                                        if (where != null)
                                          Text(
                                            where,
                                            maxLines: 1,
                                            overflow: TextOverflow.ellipsis,
                                            style: context.textTheme.bodySmall
                                                ?.copyWith(
                                                  color: context
                                                      .colors
                                                      .onSurfaceVariant,
                                                ),
                                          ),
                                      ],
                                    ),
                                  ),
                                ],
                              ),
                            ),
                          ),
                        ),
                      ),
                      if (!here)
                        Padding(
                          padding: const EdgeInsets.only(right: Grid.twelve),
                          child: IconButton(
                            icon: const Icon(BuzzIcons.activity, size: 18),
                            tooltip: 'View $name activity',
                            onPressed: () => viewActivity(pubkey),
                          ),
                        ),
                    ],
                  );
                },
              ),
          ],
        ),
      ),
    );
  }
}

/// Desktop's thread name: the root's first 60 characters on one line.
String? _preview(TimelineMessage? message) {
  final text = message?.content.replaceAll(RegExp(r'\s+'), ' ').trim();
  if (text == null || text.isEmpty) return null;
  return text.length > 60 ? text.substring(0, 60) : text;
}
