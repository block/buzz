import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/relay/relay.dart';
import 'channel.dart';
import 'bestie_transcription_draft.dart';
import 'send_message_provider.dart';

/// Sends text only, retaining the original signed event after uncertain delivery.
Future<Channel> deliverBestieTranscript(
  Ref ref,
  BestieTranscriptStore store,
  BestieTranscriptDraft draft,
  Channel channel,
  void Function() validate,
) async {
  validate();
  final session = ref.read(relaySessionProvider.notifier);
  final event = draft.event;
  if (event != null) {
    // Never re-sign an uncertain delivery. Its exact ID makes retries idempotent.
    try {
      await session.publish(event);
    } catch (failure) {
      if (!failure.toString().contains(
        'invalid: event timestamp too far from server time',
      )) {
        rethrow;
      }
      final accepted = await session.queryRelay([
        NostrFilter(kinds: [event.kind], ids: [event.id]),
      ]);
      validate();
      if (!accepted.any((candidate) => candidate.id == event.id)) {
        await SignedEventRelay(
          session: session,
          nsec: ref.read(relayConfigProvider).nsec,
        ).submit(
          kind: event.kind,
          content: event.content,
          tags: event.tags,
          onSigned: (renewed) async {
            await store.update(draft, event: renewed);
            validate();
          },
        );
      }
    }
  } else {
    final text = draft.text?.trim() ?? '';
    if (text.isEmpty) throw StateError('No speech was recognized.');
    validate();
    await ref
        .read(sendMessageProvider)
        .call(
          channelId: channel.id,
          channel: channel,
          content: text,
          mentionPubkeys: [draft.recipient],
          validateDelivery: validate,
          beforePublish: (event) async {
            await store.update(draft, event: event);
            validate();
          },
        );
  }
  await store.clear(id: draft.id);
  return channel;
}
