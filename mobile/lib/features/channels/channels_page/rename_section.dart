part of '../channels_page.dart';

/// Values submitted by the section rename editor.
typedef SectionRenameResult = ({String name, String? icon});

/// Edits a section’s name and icon together, returning null on cancellation.
Future<SectionRenameResult?> showSectionRenameEditor(
  BuildContext context,
  ChannelSection section,
) async {
  if (defaultTargetPlatform == TargetPlatform.iOS) {
    const editor = MethodChannel('buzz/profile_text_editor');
    const picker = MethodChannel('buzz/section_emoji');
    final theme = utilitySheetThemeData(Theme.of(context));
    final container = ProviderScope.containerOf(context, listen: false);
    Map<String, Object?> selection(String icon) {
      if (!icon.startsWith(':')) return {'icon': icon};
      final url = container
          .read(customEmojiListProvider)
          .where((e) => ':${e.shortcode}:' == icon)
          .firstOrNull
          ?.url;
      return {
        'icon': icon,
        if (url != null)
          'iconBytes': container
              .read(customEmojiBytesCacheProvider)
              .lookup(url),
      };
    }

    var editorOpen = true;
    picker.setMethodCallHandler((call) async {
      if (call.method != 'choose' || !context.mounted) return null;
      var unicodeOnly = false;
      try {
        await container.read(customEmojiPaletteProvider.future);
      } catch (_) {
        unicodeOnly = true;
      }
      if (!editorOpen || !context.mounted) return null;
      final result = Completer<Map<String, Object?>?>();
      showEmojiPicker(
        context: context,
        nativeEmojiOnly: unicodeOnly,
        onSelect: (value) {
          if (!result.isCompleted) result.complete(selection(value));
        },
        onDismiss: () {
          if (!result.isCompleted) result.complete(null);
        },
      );
      return result.future;
    });
    try {
      final value = await editor
          .invokeMapMethod<String, dynamic>('presentSection', {
            'title': 'Rename section',
            'initialValue': section.name,
            'placeholder': 'Section name',
            'multiline': false,
            'requireNonEmpty': true,
            'brightness': theme.brightness.name,
            'pageBackgroundArgb': theme.colorScheme.surface.toARGB32(),
            'containerBackgroundArgb': theme.colorScheme.surfaceContainerHighest
                .toARGB32(),
            'containerCornerRadius': Radii.container,
            ...selection(section.icon ?? ''),
          });
      if (value == null) return null;
      final name = (value['name'] as String).trim();
      final icon = value['icon'] as String;
      return name.isEmpty
          ? null
          : (name: name, icon: icon.isEmpty ? null : icon);
    } on MissingPluginException {
      // Older iOS builds use the same editable fields in Flutter.
    } on PlatformException {
      // A presentation failure still allows editing in Flutter.
    } finally {
      editorOpen = false;
      picker.setMethodCallHandler(null);
    }
  }
  if (!context.mounted) return null;
  return showBuzzDialog<SectionRenameResult>(
    context: context,
    builder: (_) => _SectionNameDialog(section),
  );
}

class _SectionNameDialog extends HookWidget {
  const _SectionNameDialog(this.section);
  final ChannelSection section;

  @override
  Widget build(BuildContext context) {
    final controller = useTextEditingController(text: section.name);
    final icon = useState(section.icon);
    void confirm() {
      final name = controller.text.trim();
      if (name.isNotEmpty) {
        Navigator.of(context).pop((name: name, icon: icon.value));
      }
    }

    return AlertDialog(
      scrollable: true,
      title: const Text('Rename Section'),
      content: Row(
        children: [
          SizedBox.square(
            dimension: 52,
            child: IconButton.filledTonal(
              tooltip: 'Choose section emoji',
              icon: icon.value == null
                  ? const Icon(Icons.emoji_emotions_outlined)
                  : _WorkspaceIcon(icon.value!),
              onPressed: () => showEmojiPicker(
                context: context,
                onSelect: (value) {
                  if (context.mounted) icon.value = value;
                },
              ),
            ),
          ),
          const SizedBox(width: 8),
          Expanded(
            child: ConstrainedBox(
              constraints: const BoxConstraints(minHeight: 52),
              child: TextField(
                controller: controller,
                autofocus: true,
                decoration: const InputDecoration(labelText: 'Name'),
                onSubmitted: (_) => confirm(),
              ),
            ),
          ),
        ],
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: const Text('Cancel'),
        ),
        TextButton(onPressed: confirm, child: const Text('Rename')),
      ],
    );
  }
}
