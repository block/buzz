import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../../shared/theme/theme.dart';
import 'channel.dart';
import 'channel_management_provider.dart';
import 'channel_mutes/channel_mutes_provider.dart';

class ManageChannelSheet extends HookConsumerWidget {
  final Channel channel;

  const ManageChannelSheet({super.key, required this.channel});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final canvasAsync = ref.watch(channelCanvasProvider(channel.id));
    final isEditingCanvas = useState(false);
    final isSavingCanvas = useState(false);
    final isBusy = useState(false);
    final actionError = useState<String?>(null);
    final canvasController = useTextEditingController();

    useEffect(() {
      final canvas = canvasAsync.asData?.value;
      if (!isEditingCanvas.value) {
        canvasController.text = canvas?.content ?? '';
      }
      return null;
    }, [canvasAsync.asData?.value.content, isEditingCanvas.value]);

    final mutesState = ref.watch(channelMutesProvider);
    final isMuted = mutesState.store.channels[channel.id]?.muted == true;

    final canJoin =
        channel.visibility == 'open' &&
        !channel.isArchived &&
        !channel.isMember &&
        !channel.isDm;
    final canLeave = channel.isMember && !channel.isArchived && !channel.isDm;
    final canEditCanvas = channel.isMember && !channel.isArchived;

    Future<void> joinChannel() async {
      if (isBusy.value) return;
      isBusy.value = true;
      actionError.value = null;
      try {
        await ref.read(channelActionsProvider).joinChannel(channel.id);
        if (context.mounted) {
          Navigator.of(context).pop(false);
        }
      } catch (error) {
        actionError.value = error.toString();
      } finally {
        isBusy.value = false;
      }
    }

    Future<void> leaveChannel() async {
      if (isBusy.value) return;
      isBusy.value = true;
      actionError.value = null;
      try {
        await ref.read(channelActionsProvider).leaveChannel(channel.id);
        if (context.mounted) {
          Navigator.of(context).pop(true);
        }
      } catch (error) {
        actionError.value = error.toString();
      } finally {
        isBusy.value = false;
      }
    }

    Future<void> saveCanvas() async {
      if (isSavingCanvas.value) {
        return;
      }
      isSavingCanvas.value = true;
      actionError.value = null;
      try {
        await ref
            .read(channelActionsProvider)
            .setCanvas(
              channelId: channel.id,
              content: canvasController.text.trim(),
            );
        if (context.mounted) {
          isEditingCanvas.value = false;
        }
      } catch (error) {
        actionError.value = error.toString();
      } finally {
        isSavingCanvas.value = false;
      }
    }

    return Padding(
      padding: EdgeInsets.fromLTRB(
        Grid.xs,
        0,
        Grid.xs,
        MediaQuery.viewInsetsOf(context).bottom + Grid.xs,
      ),
      child: SafeArea(
        top: false,
        child: ListView(
          shrinkWrap: true,
          children: [
            Text('Manage channel', style: context.textTheme.titleMedium),
            const SizedBox(height: Grid.xxs),
            Text(
              'Basic management for ${channel.name}.',
              style: context.textTheme.bodySmall?.copyWith(
                color: context.colors.onSurfaceVariant,
              ),
            ),
            if (actionError.value case final error?) ...[
              const SizedBox(height: Grid.xs),
              Text(
                error,
                style: context.textTheme.bodySmall?.copyWith(
                  color: context.colors.error,
                ),
              ),
            ],
            const SizedBox(height: Grid.xs),
            SwitchListTile(
              title: const Text('Mute channel'),
              subtitle: const Text('Suppress notifications and unread badges'),
              secondary: const Icon(LucideIcons.bellOff),
              contentPadding: EdgeInsets.zero,
              value: isMuted,
              onChanged: (value) {
                if (value) {
                  ref
                      .read(channelMutesProvider.notifier)
                      .muteChannel(channel.id);
                } else {
                  ref
                      .read(channelMutesProvider.notifier)
                      .unmuteChannel(channel.id);
                }
              },
            ),
            if (canJoin || canLeave) ...[
              const SizedBox(height: Grid.xs),
              Wrap(
                spacing: Grid.xxs,
                children: [
                  if (canJoin)
                    FilledButton.tonal(
                      onPressed: isBusy.value ? null : joinChannel,
                      child: Text(
                        isBusy.value ? 'Joining\u2026' : 'Join channel',
                      ),
                    ),
                  if (canLeave)
                    OutlinedButton(
                      onPressed: isBusy.value ? null : leaveChannel,
                      child: Text(
                        isBusy.value ? 'Leaving\u2026' : 'Leave channel',
                      ),
                    ),
                ],
              ),
            ],
            const SizedBox(height: Grid.sm),
            Text('Context', style: context.textTheme.labelLarge),
            const SizedBox(height: Grid.xxs),
            _ContextCard(
              label: 'Description',
              value: channel.description,
              emptyLabel: 'No description set',
            ),
            const SizedBox(height: Grid.xxs),
            _ContextCard(
              label: 'Topic',
              value: channel.topic,
              emptyLabel: 'No topic set',
            ),
            const SizedBox(height: Grid.xxs),
            _ContextCard(
              label: 'Purpose',
              value: channel.purpose,
              emptyLabel: 'No purpose set',
            ),
            if (!channel.isDm) ...[
              const SizedBox(height: Grid.sm),
              Text('Canvas', style: context.textTheme.labelLarge),
              const SizedBox(height: Grid.xxs),
              canvasAsync.when(
                data: (canvas) {
                  if (isEditingCanvas.value) {
                    return Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        TextField(
                          controller: canvasController,
                          maxLines: 8,
                          minLines: 6,
                          decoration: const InputDecoration(
                            hintText:
                                'Write your canvas content in Markdown\u2026',
                          ),
                        ),
                        const SizedBox(height: Grid.xxs),
                        Row(
                          mainAxisAlignment: MainAxisAlignment.end,
                          children: [
                            TextButton(
                              onPressed: isSavingCanvas.value
                                  ? null
                                  : () {
                                      isEditingCanvas.value = false;
                                      canvasController.text =
                                          canvas.content ?? '';
                                    },
                              child: const Text('Cancel'),
                            ),
                            const SizedBox(width: Grid.half),
                            FilledButton(
                              onPressed: isSavingCanvas.value
                                  ? null
                                  : saveCanvas,
                              child: Text(
                                isSavingCanvas.value
                                    ? 'Saving\u2026'
                                    : 'Save canvas',
                              ),
                            ),
                          ],
                        ),
                      ],
                    );
                  }

                  return Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Container(
                        width: double.infinity,
                        padding: const EdgeInsets.all(Grid.xs),
                        decoration: BoxDecoration(
                          color: context.colors.surfaceContainerHighest,
                          borderRadius: BorderRadius.circular(Radii.md),
                        ),
                        child: Text(
                          canvas.content?.trim().isNotEmpty == true
                              ? canvas.content!
                              : 'No canvas set for this channel.',
                          style: context.textTheme.bodyMedium?.copyWith(
                            color: context.colors.onSurfaceVariant,
                          ),
                        ),
                      ),
                      const SizedBox(height: Grid.xxs),
                      Align(
                        alignment: Alignment.centerRight,
                        child: FilledButton.tonal(
                          onPressed: canEditCanvas
                              ? () => isEditingCanvas.value = true
                              : null,
                          child: Text(
                            canvas.content?.trim().isNotEmpty == true
                                ? 'Edit canvas'
                                : 'Create canvas',
                          ),
                        ),
                      ),
                    ],
                  );
                },
                loading: () => const Center(child: CircularProgressIndicator()),
                error: (error, _) => Text(
                  error.toString(),
                  style: context.textTheme.bodySmall?.copyWith(
                    color: context.colors.error,
                  ),
                ),
              ),
            ],
          ],
        ),
      ),
    );
  }
}

class _ContextCard extends StatelessWidget {
  final String label;
  final String? value;
  final String emptyLabel;

  const _ContextCard({
    required this.label,
    required this.value,
    required this.emptyLabel,
  });

  @override
  Widget build(BuildContext context) {
    return Container(
      width: double.infinity,
      padding: const EdgeInsets.all(Grid.xs),
      decoration: BoxDecoration(
        color: context.colors.surfaceContainerHighest,
        borderRadius: BorderRadius.circular(Radii.md),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(
            label,
            style: context.textTheme.labelSmall?.copyWith(
              color: context.colors.onSurfaceVariant,
              fontWeight: FontWeight.w600,
            ),
          ),
          const SizedBox(height: Grid.half),
          Text(
            value?.trim().isNotEmpty == true ? value!.trim() : emptyLabel,
            style: context.textTheme.bodyMedium?.copyWith(
              color: context.colors.onSurfaceVariant,
            ),
          ),
        ],
      ),
    );
  }
}
