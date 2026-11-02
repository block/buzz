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
    final request = useMemoized(() => controller.load(channel.id), [
      controller,
      sync,
      channel.id,
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

    Future<void> convert() async {
      if (busy.value || !canConvert || visibility == null || data == null) {
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
        final confirmed = await showDialog<bool>(
          context: context,
          builder: (dialogContext) => AlertDialog(
            title: const Text('Convert this chat?'),
            content: Text(
              'The same history, Canvas and people will become a ${visibility == 'private' ? 'private' : 'community-readable'} channel. This chat will leave your Solo workspace.',
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
            if (ref.read(channelAccessProvider).containsKey(channel.id)) {
              await controller.checkSave(channel.id);
            }
            final fresh = await controller.load(channel.id);
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
            } else if (fresh.data.visibility != selected.visibility ||
                fresh.data.joinRole != selected.joinRole) {
              throw StateError(
                'The converted channel has different access. Check its settings.',
              );
            }
          },
        );
        if (context.mounted) Navigator.pop(context);
      } catch (reason) {
        if (context.mounted) error.value = reason.toString();
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
              FilledButton(
                onPressed: busy.value || !canConvert ? null : convert,
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
