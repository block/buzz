import 'package:flutter/material.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../../shared/theme/theme.dart';
import 'observer_models.dart';
import 'observer_subscription.dart';
import 'inline_activity_item.dart';
import 'work_snapshot.dart';

/// The same authorized live stream as the activity sheet, contained in chat.
/// Mounted only while the disclosure is open.
class InlineLiveActivity extends ConsumerWidget {
  const InlineLiveActivity({
    super.key,
    required this.channelId,
    required this.agentPubkey,
    this.threadRoots = const {null},
  });

  final String channelId;
  final String agentPubkey;
  final Set<String?> threadRoots;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final state = ref.watch(
      observerSubscriptionProvider((
        channelId: channelId,
        agentPubkey: agentPubkey,
      )),
    );
    final snapshot = currentWorkSnapshot(
      state.workSnapshots,
      threadRoots,
      DateTime.now().millisecondsSinceEpoch,
    );
    final items = snapshot == null
        ? <TranscriptItem>[]
        : [
            for (var index = 0; index < snapshot.steps.length; index++)
              MessageItem(
                id: '${snapshot.turnId}:$index',
                role: 'assistant',
                title: '',
                text: snapshot.steps[index],
                timestamp: '',
              ),
          ];
    return Padding(
      padding: const EdgeInsets.fromLTRB(
        Grid.xs,
        Grid.twelve,
        Grid.xs,
        Grid.half,
      ),
      child: items.isEmpty
          ? Text(
              snapshot != null
                  ? 'Starting…'
                  : switch (state.connection) {
                      ObserverConnectionState.error =>
                        'Live activity could not connect. ${state.errorMessage ?? ''}',
                      ObserverConnectionState.idle =>
                        'Live activity is not connected.',
                      _ => 'Waiting for live activity…',
                    },
            )
          : ConstrainedBox(
              constraints: const BoxConstraints(maxHeight: 320),
              child: ListView.builder(
                primary: false,
                shrinkWrap: true,
                itemCount: items.length,
                itemBuilder: (context, index) =>
                    InlineActivityItem(item: items[index]),
              ),
            ),
    );
  }
}
