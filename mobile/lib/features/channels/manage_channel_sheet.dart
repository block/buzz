import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/theme/theme.dart';
import '../../shared/widgets/modal_presentation.dart';
import '../../shared/widgets/text_editor.dart';
import 'channel.dart';
import 'channel_management_provider.dart';

String _channelName(String value) =>
    value.trim().replaceFirst(RegExp(r'^#+'), '').trim();

/// Opens name and description directly in one full editing sheet.
Future<bool?> showManageChannelSheet({
  required BuildContext context,
  required Channel channel,
  required bool canEditDetails,
  ValueChanged<Channel>? onChannelUpdated,
}) async {
  final container = ProviderScope.containerOf(context, listen: false);
  final actions = container.read(channelActionsProvider);
  var nameDraft = channel.name;
  var descriptionDraft = channel.description;
  var retry = false;
  final canEditCanvas = channel.isMember && !channel.isArchived;
  bool canPresent() =>
      context.mounted && (ModalRoute.of(context)?.isCurrent ?? true);

  if (defaultTargetPlatform == TargetPlatform.iOS) {
    final theme = utilitySurfaceThemeData(Theme.of(context));
    const bridge = MethodChannel('buzz/profile_text_editor');
    // Start the provider without delaying metadata editing on relay I/O.
    // A cold or failed canvas can be loaded/retried from its row.
    final cachedCanvas = container
        .read(channelCanvasProvider(channel.id))
        .asData;
    String? canvasContent = cachedCanvas?.value.content ?? '';
    var canvasLoaded = cachedCanvas != null;
    while (canPresent()) {
      Map<String, dynamic>? result;
      try {
        result = await bridge
            .invokeMapMethod<String, dynamic>('presentChannel', {
              'title': 'Edit channel',
              'initialValue': nameDraft,
              'description': descriptionDraft,
              'originalName': channel.name,
              'originalDescription': channel.description,
              'placeholder': 'Channel name',
              'multiline': false,
              'brightness': theme.brightness.name,
              'pageBackgroundArgb': theme.colorScheme.surface.toARGB32(),
              'containerBackgroundArgb': theme
                  .colorScheme
                  .surfaceContainerHighest
                  .toARGB32(),
              'containerCornerRadius': Radii.container,
              'allowUnchangedSubmission': retry,
              'canEditDetails': canEditDetails,
              'canEditCanvas': canEditCanvas,
              'canvasContent': canvasContent,
              'canvasLoaded': canvasLoaded,
            });
      } on MissingPluginException {
        break;
      } on PlatformException {
        break;
      }
      if (result == null || !context.mounted || !canPresent()) return false;
      nameDraft = result['name'] as String? ?? nameDraft;
      descriptionDraft = result['description'] as String? ?? descriptionDraft;
      try {
        if (result['action'] == 'canvas') {
          if (canEditCanvas) {
            final saved = await _editCanvas(
              context,
              container,
              actions,
              channel.id,
              initialContent: canvasLoaded ? canvasContent : null,
            );
            if (saved != null) {
              canvasContent = saved;
              canvasLoaded = true;
            }
          }
          continue;
        }
        if (result['action'] == 'removeCanvas') {
          if (canEditCanvas &&
              canvasLoaded &&
              (canvasContent?.trim().isNotEmpty ?? false)) {
            await actions.setCanvas(channelId: channel.id, content: '');
            canvasContent = '';
          }
          continue;
        }
        if (result['action'] != 'save' || !canEditDetails) return false;
        final updated = await _saveDetails(
          actions,
          channel,
          nameDraft,
          descriptionDraft,
        );
        if (context.mounted) onChannelUpdated?.call(updated);
        return false;
      } catch (_) {
        if (!context.mounted || !canPresent()) return false;
        retry = true;
        ScaffoldMessenger.of(context).showSnackBar(
          const SnackBar(
            content: Text("We couldn't save this change. Try again."),
          ),
        );
      }
    }
  }
  if (!context.mounted || !canPresent()) return false;
  return showBuzzModalBottomSheet<bool>(
    context: context,
    title: 'Edit channel',
    isScrollControlled: true,
    useSafeArea: true,
    constraints: const BoxConstraints(maxWidth: 640),
    builder: (_) => SizedBox(
      height: MediaQuery.sizeOf(context).height * 0.9,
      child: ManageChannelSheet(
        channel: channel,
        canEditDetails: canEditDetails,
        onChannelUpdated: onChannelUpdated,
        initialName: nameDraft,
        initialDescription: descriptionDraft,
      ),
    ),
  );
}

Future<Channel> _saveDetails(
  ChannelActions actions,
  Channel channel,
  String name,
  String description,
) async {
  final canonicalName = _channelName(name);
  final trimmedDescription = description.trim();
  if (canonicalName.isEmpty) {
    throw const FormatException('Channel name is required.');
  }
  await actions.updateChannel(
    channelId: channel.id,
    name: canonicalName != channel.name.trim() ? canonicalName : null,
    description: trimmedDescription != channel.description.trim()
        ? trimmedDescription
        : null,
  );
  return channel.copyWith(name: canonicalName, description: trimmedDescription);
}

Future<String?> _editCanvas(
  BuildContext context,
  ProviderContainer container,
  ChannelActions actions,
  String channelId, {
  String? initialContent,
}) async {
  if (initialContent == null &&
      container.read(channelCanvasProvider(channelId)).hasError) {
    container.invalidate(channelCanvasProvider(channelId));
  }
  final content =
      initialContent ??
      (await container
              .read(channelCanvasProvider(channelId).future)
              .timeout(const Duration(seconds: 5)))
          .content ??
      '';
  if (!context.mounted) return null;
  String? saved;
  await showTextEditor(
    context: context,
    title: 'Canvas',
    initialValue: content,
    hintText: 'Write your canvas content in Markdown…',
    multiline: true,
    onSave: (value) async {
      final trimmed = value.trim();
      await actions.setCanvas(channelId: channelId, content: trimmed);
      saved = trimmed;
    },
  );
  // A successful load is also useful when the editor is dismissed unchanged.
  return saved ?? content;
}

/// Flutter fallback for the full channel editor.
class ManageChannelSheet extends HookConsumerWidget {
  const ManageChannelSheet({
    super.key,
    required this.channel,
    this.canEditDetails = true,
    this.onChannelUpdated,
    this.initialName,
    this.initialDescription,
  });

  final Channel channel;
  final bool canEditDetails;
  final ValueChanged<Channel>? onChannelUpdated;
  final String? initialName;
  final String? initialDescription;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final name = useTextEditingController(text: initialName ?? channel.name);
    final description = useTextEditingController(
      text: initialDescription ?? channel.description,
    );
    useListenable(name);
    useListenable(description);
    final saving = useState(false);
    final openingCanvas = useState(false);
    final error = useState<String?>(null);
    final canvas = ref.watch(channelCanvasProvider(channel.id));
    final savedCanvas = useState<String?>(null);
    final canvasContent =
        savedCanvas.value ?? canvas.asData?.value.content ?? '';
    final canvasLoaded = savedCanvas.value != null || canvas.hasValue;
    final hasCanvas = canvasLoaded && canvasContent.trim().isNotEmpty;
    final canEditCanvas = channel.isMember && !channel.isArchived;
    // Keep actions bound to the community in which this editor was opened.
    final actions = useMemoized(() => ref.read(channelActionsProvider));
    final canonicalName = _channelName(name.text);
    final changed =
        canonicalName != channel.name.trim() ||
        description.text.trim() != channel.description.trim();
    final busy = saving.value || openingCanvas.value;

    Future<void> save() async {
      if (busy || !canEditDetails || !changed || canonicalName.isEmpty) return;
      saving.value = true;
      error.value = null;
      try {
        final updated = await _saveDetails(
          actions,
          channel,
          name.text,
          description.text,
        );
        if (!context.mounted) return;
        onChannelUpdated?.call(updated);
        Navigator.of(context).pop(false);
      } catch (_) {
        if (context.mounted) {
          error.value = "We couldn't save this change. Try again.";
        }
      } finally {
        if (context.mounted) saving.value = false;
      }
    }

    Future<void> editCanvas() async {
      if (busy) return;
      openingCanvas.value = true;
      error.value = null;
      try {
        final saved = await _editCanvas(
          context,
          ProviderScope.containerOf(context, listen: false),
          actions,
          channel.id,
          initialContent: canvasLoaded ? canvasContent : null,
        );
        if (context.mounted && saved != null) savedCanvas.value = saved;
      } catch (_) {
        if (context.mounted) {
          error.value = "Couldn't load the canvas. Try again.";
        }
      } finally {
        if (context.mounted) openingCanvas.value = false;
      }
    }

    Future<void> removeCanvas() async {
      if (busy || !canEditCanvas || !hasCanvas) return;
      final confirmed = await showDialog<bool>(
        context: context,
        builder: (context) => AlertDialog(
          title: const Text('Remove canvas?'),
          content: const Text(
            'This removes the canvas content for everyone in this channel.',
          ),
          actions: [
            TextButton(
              onPressed: () => Navigator.pop(context, false),
              child: const Text('Cancel'),
            ),
            TextButton(
              onPressed: () => Navigator.pop(context, true),
              child: const Text('Remove'),
            ),
          ],
        ),
      );
      if (confirmed != true || !context.mounted) return;
      openingCanvas.value = true;
      error.value = null;
      try {
        await actions.setCanvas(channelId: channel.id, content: '');
        if (context.mounted) savedCanvas.value = '';
      } catch (_) {
        if (context.mounted) {
          error.value = "Couldn't remove the canvas. Try again.";
        }
      } finally {
        if (context.mounted) openingCanvas.value = false;
      }
    }

    return SingleChildScrollView(
      padding: EdgeInsets.fromLTRB(
        Grid.gutter,
        Grid.xxs,
        Grid.gutter,
        MediaQuery.viewInsetsOf(context).bottom + Grid.xs,
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          TextField(
            key: const ValueKey('manage-channel-name'),
            controller: name,
            autofocus: canEditDetails,
            enabled: canEditDetails && !busy,
            textCapitalization: TextCapitalization.sentences,
            textInputAction: TextInputAction.next,
            decoration: const InputDecoration(hintText: 'Channel name'),
          ),
          const SizedBox(height: Grid.xs),
          TextField(
            key: const ValueKey('manage-channel-description'),
            controller: description,
            enabled: canEditDetails && !busy,
            minLines: 4,
            maxLines: 6,
            textCapitalization: TextCapitalization.sentences,
            textInputAction: TextInputAction.newline,
            decoration: const InputDecoration(hintText: 'Description'),
          ),
          const SizedBox(height: Grid.xs),
          if (hasCanvas) ...[
            Material(
              color: context.colors.surfaceContainerHighest,
              borderRadius: BorderRadius.circular(Radii.container),
              clipBehavior: Clip.antiAlias,
              child: ListTile(
                key: const ValueKey('manage-channel-canvas'),
                title: const Text('Canvas'),
                subtitle: Text(
                  canvasContent,
                  maxLines: 4,
                  overflow: TextOverflow.ellipsis,
                ),
                trailing: const Icon(Icons.chevron_right),
                enabled: canEditCanvas && !busy,
                onTap: canEditCanvas && !busy ? editCanvas : null,
              ),
            ),
            if (canEditCanvas)
              TextButton(
                key: const ValueKey('manage-channel-remove-canvas'),
                onPressed: busy ? null : removeCanvas,
                style: TextButton.styleFrom(
                  foregroundColor: context.colors.error,
                ),
                child: const Text('Remove Canvas'),
              ),
          ] else
            FilledButton.tonal(
              key: const ValueKey('manage-channel-canvas'),
              onPressed: canEditCanvas && !busy ? editCanvas : null,
              child: Text(
                openingCanvas.value || (!canvasLoaded && canvas.isLoading)
                    ? 'Loading…'
                    : canvasLoaded
                    ? 'Add Canvas'
                    : 'Retry loading canvas',
              ),
            ),
          if (canonicalName.isEmpty || error.value != null) ...[
            const SizedBox(height: Grid.xxs),
            Semantics(
              liveRegion: true,
              child: Text(
                canonicalName.isEmpty
                    ? 'Channel name is required.'
                    : error.value!,
                style: context.textTheme.bodySmall?.copyWith(
                  color: context.colors.error,
                ),
              ),
            ),
          ],
          const SizedBox(height: Grid.xs),
          FilledButton(
            key: const ValueKey('manage-channel-save-details'),
            onPressed:
                canEditDetails && changed && canonicalName.isNotEmpty && !busy
                ? save
                : null,
            child: Text(saving.value ? 'Saving…' : 'Save'),
          ),
        ],
      ),
    );
  }
}
