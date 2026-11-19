import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/me/me_sync.dart';
import '../../shared/theme/theme.dart';
import 'bestie_transcription_job.dart';

/// Keeps pending speech visible and recoverable in its destination conversation.
class BestieTranscriptionStatus extends HookConsumerWidget {
  const BestieTranscriptionStatus({super.key, required this.channelId});
  final String channelId;
  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final sync = ref.watch(meSyncProvider);
    if (sync.config.nsec == null) return const SizedBox.shrink();
    return _SignedInTranscriptionStatus(
      channelId: channelId,
      scope: sync.cacheKey,
    );
  }
}

class _SignedInTranscriptionStatus extends HookConsumerWidget {
  const _SignedInTranscriptionStatus({
    required this.channelId,
    required this.scope,
  });
  final String channelId;
  final String scope;
  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final job = ref.watch(bestieTranscriptionJobProvider(scope));
    final progress = useValueListenable(job);
    if (progress.channel?.id != channelId ||
        progress.draft == null ||
        progress.error == null) {
      return const SizedBox.shrink();
    }
    return Material(
      color: context.controlSurface,
      child: Padding(
        padding: const EdgeInsets.symmetric(
          horizontal: Grid.sm,
          vertical: Grid.xxs,
        ),
        child: Row(
          children: [
            Expanded(
              child: Semantics(
                liveRegion: true,
                child: Text(
                  progress.error != null
                      ? 'Could not send transcript. Recording saved.'
                      : '${progress.phase}…',
                ),
              ),
            ),
            if (progress.error != null) ...[
              TextButton(
                onPressed: job.busy ? null : job.discard,
                child: const Text('Discard'),
              ),
              TextButton(
                onPressed: job.busy ? null : job.retry,
                child: const Text('Retry'),
              ),
            ],
          ],
        ),
      ),
    );
  }
}
