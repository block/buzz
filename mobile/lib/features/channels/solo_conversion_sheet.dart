import 'access_error_message.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/me/me_chat_conversion.dart';
import '../../shared/me/me_sync.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/buzz_loading_indicator.dart';
import '../../shared/widgets/modal_presentation.dart';
import 'channel.dart';
import 'channel_access_provider.dart';

/// Conversion is explicit and separate from sharing; the conversation stays put.
Future<void> showSoloConversionSheet(BuildContext context, Channel channel) =>
    showBuzzModalBottomSheet<void>(
      context: context,
      title: 'Convert to channel',
      showDragHandle: true,
      isScrollControlled: true,
      builder: (_) => SoloConversionSheet(channel: channel),
    );

/// Policy selection and resumable Solo filing cleanup.
class SoloConversionSheet extends HookConsumerWidget {
  const SoloConversionSheet({super.key, required this.channel});
  final Channel channel;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final controller = ref.watch(channelAccessProvider.notifier);
    final sync = ref.watch(meSyncProvider);
    final reload = useState(0);
    final request = useMemoized(() => controller.load(channel.id), [
      controller,
      sync,
      channel.id,
      reload.value,
    ]);
    final snapshot = useFuture(request);
    final intent = useState(MeChatConversion.pending(sync, channel.id));
    final choice = useState<String?>(null);
    final busy = useState(false);
    final error = useState<String?>(null);
    final data = snapshot.data?.data;
    final currentAccess = data == null
        ? null
        : data.visibility == 'private'
        ? 'private'
        : data.joinRole == 'guest'
        ? 'view-only'
        : 'public';
    final access = intent.value == null
        ? choice.value ?? currentAccess
        : intent.value!.visibility == 'private'
        ? 'private'
        : intent.value!.joinRole == 'guest'
        ? 'view-only'
        : 'public';
    final visibility = access == null
        ? null
        : access == 'private'
        ? 'private'
        : 'open';
    final canConvert =
        snapshot.data?.canEdit(sync.pubkey) == true &&
        data?.creator == sync.pubkey;

    final canFinish =
        intent.value != null &&
        data != null &&
        !data.labels.contains('buzz.solo');

    Future<void> convert() async {
      if (busy.value ||
          (!canConvert && !canFinish) ||
          visibility == null ||
          data == null) {
        return;
      }
      final selected =
          intent.value ??
          MeChatConversion.create(
            sync,
            channel: channel.id,
            visibility: visibility,
            joinRole: access == 'view-only'
                ? 'guest'
                : access == 'public'
                ? 'member'
                : data.joinRole,
            labels: data.labels.where((label) => label != 'buzz.solo').toList(),
          );
      if (intent.value == null) {
        final confirmed = await showBuzzDialog<bool>(
          context: context,
          builder: (dialogContext) => AlertDialog(
            title: const Text('Convert this chat?'),
            content: Text(
              'The same history, Canvas and people will become a ${visibility == 'private' ? 'private' : 'community-readable'} channel. ${access == 'public' ? 'Everyone in the community can join, read and post, including to your agents.' : 'Existing participants can read and post, including to your agents.'} This chat will leave your Solo workspace.',
            ),
            actions: [
              TextButton(
                autofocus: true,
                onPressed: () => Navigator.pop(dialogContext, false),
                child: const Text('Cancel'),
              ),
              FilledButton(
                onPressed: () => Navigator.pop(dialogContext, true),
                child: const Text('Convert'),
              ),
            ],
          ),
        );
        if (confirmed != true || !context.mounted) return;
        intent.value = selected;
      }
      busy.value = true;
      error.value = null;
      try {
        await selected.complete(
          sync,
          ensureConverted: () async {
            final current = await controller.load(channel.id);
            if (!current.data.labels.contains('buzz.solo')) return;
            if (ref.read(channelAccessProvider).containsKey(channel.id)) {
              await controller.checkSave(channel.id);
            }
            final fresh = await controller.load(channel.id);
            if (!fresh.data.labels.contains('buzz.solo')) return;
            final labels = fresh.data.labels
                .where((label) => label != 'buzz.solo')
                .toList();
            if (!listEquals(labels, selected.labels)) {
              throw StateError(
                'Chat access changed. Review it before finishing conversion.',
              );
            }
            if (fresh.data.labels.contains('buzz.solo')) {
              if (selected.visibility == 'private' &&
                  fresh.data.joinRole != selected.joinRole) {
                throw StateError(
                  'The saved admission default changed. Review access before converting.',
                );
              }
              await controller.convert(
                channel.id,
                fresh,
                access: selected.visibility == 'private'
                    ? 'private'
                    : selected.joinRole == 'guest'
                    ? 'view-only'
                    : 'public',
              );
            }
          },
        );
        if (context.mounted) Navigator.pop(context);
      } catch (reason) {
        if (context.mounted) error.value = accessErrorMessage(reason);
      } finally {
        if (context.mounted) busy.value = false;
      }
    }

    Future<void> discardConversion() async {
      final reviewed = intent.value;
      if (busy.value || reviewed == null) return;
      busy.value = true;
      try {
        final confirmed = await showBuzzDialog<bool>(
          context: context,
          builder: (dialogContext) => AlertDialog(
            title: const Text('Discard this conversion?'),
            content: const Text(
              'Any channel conversion that already landed stays in place. This forgets the pending conversion and its Solo filing cleanup. Nothing will be resent or rolled back.',
            ),
            actions: [
              TextButton(
                onPressed: () => Navigator.pop(dialogContext, false),
                child: const Text('Keep conversion'),
              ),
              FilledButton(
                onPressed: () => Navigator.pop(dialogContext, true),
                child: const Text('Discard conversion'),
              ),
            ],
          ),
        );
        if (confirmed != true ||
            !context.mounted ||
            !identical(intent.value, reviewed)) {
          return;
        }
        await reviewed.discard(sync);
        if (context.mounted) {
          intent.value = null;
          choice.value = null;
          error.value = null;
          reload.value++;
        }
      } catch (reason) {
        if (context.mounted) error.value = accessErrorMessage(reason);
      } finally {
        if (context.mounted) busy.value = false;
      }
    }

    return SafeArea(
      child: Padding(
        padding: const EdgeInsets.fromLTRB(
          Grid.gutter,
          0,
          Grid.gutter,
          Grid.md,
        ),
        child: SingleChildScrollView(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              const Text(
                'Keep this conversation’s history, Canvas, people and lifetime. Conversion changes how it is organized.',
              ),
              if (data == null && !snapshot.hasError)
                const BuzzLoadingIndicator(
                  size: 24,
                  semanticLabel: 'Checking chat',
                ),
              if (data != null)
                DropdownButtonFormField<String>(
                  initialValue: access,
                  decoration: const InputDecoration(
                    labelText: 'Channel access',
                  ),
                  items: const [
                    DropdownMenuItem(value: 'private', child: Text('Private')),
                    DropdownMenuItem(
                      value: 'view-only',
                      child: Text('View only'),
                    ),
                    DropdownMenuItem(value: 'public', child: Text('Public')),
                  ],
                  onChanged: busy.value || intent.value != null
                      ? null
                      : (value) => choice.value = value,
                ),
              if (visibility == 'open')
                const Text(
                  'Everyone in this community will be able to read the full history and Canvas. Existing members keep their access.',
                ),
              if (error.value != null || snapshot.hasError)
                Semantics(
                  liveRegion: true,
                  child: Text(
                    error.value ??
                        'Current chat access could not be confirmed.',
                    style: TextStyle(color: context.colors.error),
                  ),
                ),
              if (!canConvert && !canFinish && data != null)
                const Text(
                  'Only the chat creator with owner or admin access can convert this chat.',
                ),
              if (intent.value != null)
                TextButton(
                  onPressed: busy.value ? null : discardConversion,
                  child: const Text('Discard conversion'),
                ),
              FilledButton(
                onPressed:
                    busy.value ||
                        (!canConvert && !canFinish) ||
                        visibility == null
                    ? null
                    : convert,
                child: Text(
                  busy.value
                      ? 'Converting…'
                      : intent.value == null
                      ? 'Convert to channel'
                      : 'Finish conversion',
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}
