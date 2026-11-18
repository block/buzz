import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/me/me_sync.dart';
import '../../shared/transcription/device_transcription.dart';
import 'bestie_transcription_delivery.dart';
import 'bestie_transcription_draft.dart';
import 'channel.dart';
import 'channel_management_provider.dart';

class BestieTranscriptionProgress {
  const BestieTranscriptionProgress({
    this.channel,
    this.draft,
    this.phase = '',
    this.error,
  });
  final Channel? channel;
  final BestieTranscriptDraft? draft;
  final String phase;
  final Object? error;
}

final bestieTranscriptionJobProvider =
    Provider.family<BestieTranscriptionJob, String>((ref, scope) {
      final job = BestieTranscriptionJob(
        ref,
        scope,
        ref.read(bestieTranscriptStoreProvider(scope)),
      );
      ref.onDispose(job.dispose);
      ref.listen(meSyncProvider, (_, next) {
        if (next.config.nsec == null || next.cacheKey != scope) job.abort();
      });
      return job;
    });

/// A released utterance belongs to the account/community, not the Home widget.
class BestieTranscriptionJob
    extends ValueNotifier<BestieTranscriptionProgress> {
  BestieTranscriptionJob(this.ref, this.scope, this.store)
    : super(const BestieTranscriptionProgress());
  final Ref ref;
  final String scope;
  final BestieTranscriptStore store;
  DeviceTranscription? _speech;
  bool _disposed = false;
  int _generation = 0;
  bool get busy => value.phase.isNotEmpty;
  void _validate(int generation) {
    if (_disposed ||
        generation != _generation ||
        ref.read(meSyncProvider).config.nsec == null ||
        ref.read(meSyncProvider).cacheKey != scope) {
      throw StateError('The account or community changed.');
    }
  }

  Future<Channel> destination(BestieTranscriptDraft draft) =>
      ref.read(channelActionsProvider).openDm(pubkeys: [draft.recipient]);

  /// Returns true only after this attempt confirms successful delivery.
  Future<bool> run(BestieTranscriptDraft draft, Channel channel) async {
    if (busy) return false;
    final generation = ++_generation;
    var saved = draft;
    value = BestieTranscriptionProgress(
      channel: channel,
      draft: saved,
      phase: 'Transcribing',
    );
    DeviceTranscription? native;
    try {
      _validate(generation);
      if (saved.text == null && saved.event == null) {
        native = ref.read(deviceTranscriptionProvider)();
        _speech = native;
        final text = await native
            .transcribe(saved.audioPath)
            .timeout(const Duration(seconds: 65));
        _validate(generation);
        if (text.trim().isEmpty) {
          throw StateError(
            'No speech recognized. Retry or discard the recording.',
          );
        }
        saved = await store.update(saved, text: text);
      }
      _validate(generation);
      value = BestieTranscriptionProgress(
        channel: channel,
        draft: saved,
        phase: 'Sending',
      );
      await deliverBestieTranscript(
        ref,
        store,
        saved,
        channel,
        () => _validate(generation),
      );
      _validate(generation);
      value = const BestieTranscriptionProgress();
      return true;
    } catch (error) {
      try {
        saved = await store.read() ?? saved;
      } catch (_) {
        /* Keep recovery. */
      }
      if (!_disposed && generation == _generation) {
        value = BestieTranscriptionProgress(
          channel: channel,
          draft: saved,
          error: error,
        );
      }
      return false;
    } finally {
      if (identical(_speech, native)) _speech = null;
      try {
        await native?.cancel();
      } catch (_) {
        /* Already stopped. */
      }
    }
  }

  Future<void> retry() async {
    final draft = value.draft;
    final channel = value.channel;
    if (draft != null && channel != null) await run(draft, channel);
  }

  Future<void> discard() async {
    if (busy) return;
    final pending = value;
    final generation = ++_generation;
    value = BestieTranscriptionProgress(
      channel: pending.channel,
      draft: pending.draft,
      phase: 'Discarding',
    );
    try {
      await store.clear(id: pending.draft?.id);
      if (!_disposed && generation == _generation) {
        value = const BestieTranscriptionProgress();
      }
    } catch (error) {
      if (!_disposed && generation == _generation) {
        value = BestieTranscriptionProgress(
          channel: pending.channel,
          draft: pending.draft,
          error: error,
        );
      }
    }
  }

  void abort() {
    _generation++;
    unawaited(_speech?.cancel().catchError((Object _) {}));
    _speech = null;
    if (!_disposed) value = const BestieTranscriptionProgress();
  }

  @override
  void dispose() {
    abort();
    _disposed = true;
    super.dispose();
  }
}
