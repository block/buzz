import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/theme/theme.dart';
import '../../shared/widgets/buzz_loading_indicator.dart';
import 'channel.dart';
import 'channel_management_provider.dart';

class ManageChannelSheet extends HookConsumerWidget {
  const ManageChannelSheet({
    super.key,
    required this.channel,
    this.canEditDetails = true,
    this.onChannelUpdated,
  });

  final Channel channel;
  final bool canEditDetails;
  final ValueChanged<Channel>? onChannelUpdated;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final canvasAsync = ref.watch(channelCanvasProvider(channel.id));
    final nameController = useTextEditingController(text: channel.name);
    final descriptionController = useTextEditingController(
      text: channel.description,
    );
    useListenable(nameController);
    useListenable(descriptionController);
    final canvasController = useTextEditingController();
    final isEditingCanvas = useState(false);
    final isSavingCanvas = useState(false);
    final isSavingDetails = useState(false);
    final actionError = useState<String?>(null);

    useEffect(() {
      final canvas = canvasAsync.asData?.value;
      if (!isEditingCanvas.value) {
        canvasController.text = canvas?.content ?? '';
      }
      return null;
    }, [canvasAsync.asData?.value.content, isEditingCanvas.value]);

    final canonicalName = nameController.text
        .trim()
        .replaceFirst(RegExp(r'^#+'), '')
        .trim();
    final description = descriptionController.text.trim();
    final nameDirty = canonicalName != channel.name.trim();
    final descriptionDirty = description != channel.description.trim();
    final canSaveDetails =
        canEditDetails &&
        canonicalName.isNotEmpty &&
        (nameDirty || descriptionDirty) &&
        !isSavingDetails.value;
    final canEditCanvas = channel.isMember && !channel.isArchived;

    Future<void> saveDetails() async {
      if (!canSaveDetails) return;
      isSavingDetails.value = true;
      actionError.value = null;
      try {
        await ref
            .read(channelActionsProvider)
            .updateChannel(
              channelId: channel.id,
              name: nameDirty ? canonicalName : null,
              description: descriptionDirty ? description : null,
            );
        final updated = channel.copyWith(
          name: canonicalName,
          description: description,
        );
        onChannelUpdated?.call(updated);
        if (context.mounted) Navigator.of(context).pop(false);
      } catch (error) {
        actionError.value = error.toString();
      } finally {
        isSavingDetails.value = false;
      }
    }

    Future<void> saveCanvas() async {
      if (isSavingCanvas.value) return;
      isSavingCanvas.value = true;
      actionError.value = null;
      try {
        await ref
            .read(channelActionsProvider)
            .setCanvas(
              channelId: channel.id,
              content: canvasController.text.trim(),
            );
        if (context.mounted) isEditingCanvas.value = false;
      } catch (error) {
        actionError.value = error.toString();
      } finally {
        isSavingCanvas.value = false;
      }
    }

    return Padding(
      padding: EdgeInsets.fromLTRB(
        Grid.gutter,
        0,
        Grid.gutter,
        MediaQuery.viewInsetsOf(context).bottom + Grid.xs,
      ),
      child: SafeArea(
        top: false,
        child: ListView(
          shrinkWrap: true,
          children: [
            _ManageChannelTextField(
              fieldKey: const ValueKey('manage-channel-name'),
              outlineKey: const ValueKey('manage-channel-name-outline'),
              controller: nameController,
              enabled: canEditDetails && !isSavingDetails.value,
              hintText: 'Channel name',
              textInputAction: TextInputAction.next,
            ),
            const SizedBox(height: Grid.xs),
            _ManageChannelTextField(
              fieldKey: const ValueKey('manage-channel-description'),
              outlineKey: const ValueKey('manage-channel-description-outline'),
              controller: descriptionController,
              enabled: canEditDetails && !isSavingDetails.value,
              hintText: 'Description',
              minLines: 2,
              maxLines: 4,
              textInputAction: TextInputAction.newline,
            ),
            if (canonicalName.isEmpty) ...[
              const SizedBox(height: Grid.half),
              Text(
                'Channel name is required.',
                style: context.textTheme.bodySmall?.copyWith(
                  color: context.colors.error,
                ),
              ),
            ],
            const SizedBox(height: Grid.xs),
            Align(
              alignment: Alignment.centerRight,
              child: FilledButton(
                key: const ValueKey('manage-channel-save-details'),
                onPressed: canSaveDetails ? saveDetails : null,
                child: Text(isSavingDetails.value ? 'Saving…' : 'Save changes'),
              ),
            ),
            const SizedBox(height: Grid.sm),
            _ManageChannelSection(
              label: 'Canvas',
              child: canvasAsync.when(
                data: (canvas) {
                  if (isEditingCanvas.value) {
                    return Column(
                      crossAxisAlignment: CrossAxisAlignment.stretch,
                      children: [
                        TextField(
                          key: const ValueKey('manage-channel-canvas'),
                          controller: canvasController,
                          maxLines: 8,
                          minLines: 6,
                          decoration: const InputDecoration(
                            hintText: 'Write your canvas content in Markdown…',
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
                                    ? 'Saving…'
                                    : 'Save canvas',
                              ),
                            ),
                          ],
                        ),
                      ],
                    );
                  }

                  return Column(
                    crossAxisAlignment: CrossAxisAlignment.stretch,
                    children: [
                      Text(
                        canvas.content?.trim().isNotEmpty == true
                            ? canvas.content!
                            : 'No canvas set for this channel.',
                        style: context.textTheme.bodyMedium?.copyWith(
                          color: context.colors.onSurfaceVariant,
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
                loading: () => const Center(
                  child: BuzzLoadingIndicator(
                    size: 40,
                    semanticLabel: 'Loading channel canvas',
                  ),
                ),
                error: (error, _) => Text(
                  'Couldn’t load the channel canvas.',
                  style: context.textTheme.bodySmall?.copyWith(
                    color: context.colors.error,
                  ),
                ),
              ),
            ),
            if (actionError.value case final error?) ...[
              const SizedBox(height: Grid.xxs),
              Text(
                error,
                style: context.textTheme.bodySmall?.copyWith(
                  color: context.colors.error,
                ),
              ),
            ],
          ],
        ),
      ),
    );
  }
}

class _ManageChannelTextField extends StatelessWidget {
  const _ManageChannelTextField({
    required this.fieldKey,
    required this.outlineKey,
    required this.controller,
    required this.enabled,
    required this.hintText,
    required this.textInputAction,
    this.minLines = 1,
    this.maxLines = 1,
  });

  final Key fieldKey;
  final Key outlineKey;
  final TextEditingController controller;
  final bool enabled;
  final String hintText;
  final TextInputAction textInputAction;
  final int minLines;
  final int maxLines;

  @override
  Widget build(BuildContext context) {
    return DecoratedBox(
      key: outlineKey,
      decoration: BoxDecoration(
        border: Border.all(
          color: context.colors.outlineVariant.withValues(alpha: 0.8),
        ),
        borderRadius: BorderRadius.circular(Radii.card),
      ),
      child: Semantics(
        label: hintText,
        textField: true,
        child: TextField(
          key: fieldKey,
          controller: controller,
          enabled: enabled,
          minLines: minLines,
          maxLines: maxLines,
          style: context.textTheme.bodyLarge,
          decoration: InputDecoration(
            hintText: hintText,
            hintStyle: context.textTheme.bodyLarge?.copyWith(
              color: context.colors.onSurfaceVariant,
            ),
            border: InputBorder.none,
            enabledBorder: InputBorder.none,
            focusedBorder: InputBorder.none,
            isDense: true,
            contentPadding: const EdgeInsets.symmetric(
              horizontal: Grid.xs,
              vertical: Grid.twelve,
            ),
          ),
          textInputAction: textInputAction,
        ),
      ),
    );
  }
}

class _ManageChannelSection extends StatelessWidget {
  const _ManageChannelSection({required this.label, required this.child});

  final String label;
  final Widget child;

  @override
  Widget build(BuildContext context) => Column(
    crossAxisAlignment: CrossAxisAlignment.start,
    children: [
      Padding(
        padding: const EdgeInsets.only(left: Grid.half, bottom: Grid.xxs),
        child: Text(
          label,
          style: context.textTheme.labelMedium?.copyWith(
            color: context.colors.onSurfaceVariant,
            fontWeight: FontWeight.w600,
          ),
        ),
      ),
      Material(
        color: context.colors.surfaceContainerHighest,
        borderRadius: BorderRadius.circular(Radii.card),
        clipBehavior: Clip.antiAlias,
        child: Padding(padding: const EdgeInsets.all(Grid.xs), child: child),
      ),
    ],
  );
}
