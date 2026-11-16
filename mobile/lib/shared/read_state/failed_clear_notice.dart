import 'package:flutter/material.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../widgets/feedback_snackbar.dart';
import 'read_state_provider.dart';

/// Reports, on any screen, a synced clear that failed and so left a channel
/// unread: once per newly failed channel, never again while it stands.
class FailedClearNotice extends ConsumerWidget {
  const FailedClearNotice({super.key, required this.child});

  final Widget child;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    ref.listen(readStateProvider.select((s) => s.failedClears), (
      previous,
      failed,
    ) {
      if (failed.difference(previous ?? const {}).isEmpty) return;
      ScaffoldMessenger.maybeOf(context)?.showFeedbackSnackBar(
        const SnackBar(
          content: Text('Couldn’t sync marking a channel as read. Try again.'),
        ),
        kind: ToastKind.error,
      );
    });
    return child;
  }
}
