part of '../compose_bar.dart';

/// Formatting belongs to the selected text, alongside the platform edit actions.
class _ComposerSelectionMenu extends HookWidget {
  const _ComposerSelectionMenu({
    required this.editable,
    required this.onFormat,
    required this.clipboardHasImage,
    required this.onPasteImage,
  });
  final EditableTextState editable;
  final void Function(String, [String?]) onFormat;
  final bool clipboardHasImage;
  final VoidCallback onPasteImage;

  @override
  Widget build(BuildContext context) {
    final formatting = useState(false);
    final pendingFormat = useRef(false);
    final nativeController = useMemoized(
      () => SystemContextMenuController(
        onSystemHide: () {
          if (!context.mounted) return;
          if (pendingFormat.value) {
            pendingFormat.value = false;
            formatting.value = true;
          } else {
            editable.hideToolbar(false);
          }
        },
      ),
    );
    useEffect(() => nativeController.dispose, [nativeController]);
    final value = editable.textEditingValue;
    final canFormat =
        !editable.widget.readOnly &&
        value.selection.isValid &&
        !value.selection.isCollapsed;
    const formats = [
      ('Bold', '**', '**'),
      ('Italic', '_', '_'),
      ('Strikethrough', '~~', '~~'),
      ('Code', '`', '`'),
      ('Code block', '```\n', '\n```'),
    ];
    void apply(String prefix, String suffix) {
      editable.hideToolbar(false);
      if (!editable.mounted || editable.textEditingValue != value) return;
      onFormat(prefix, suffix);
    }

    final native =
        defaultTargetPlatform == TargetPlatform.iOS &&
        SystemContextMenu.isSupportedByField(editable);
    if (native) {
      final menu = SystemContextMenu.editableText(
        editableTextState: editable,
        items: formatting.value && canFormat
            ? [
                for (final style in formats)
                  IOSSystemContextMenuItemCustom(
                    title: style.$1,
                    onPressed: () => apply(style.$2, style.$3),
                  ),
              ]
            : [
                if (canFormat)
                  IOSSystemContextMenuItemCustom(
                    title: 'Format',
                    onPressed: () => pendingFormat.value = true,
                  ),
                if (clipboardHasImage)
                  IOSSystemContextMenuItemCustom(
                    title: 'Paste Image',
                    onPressed: onPasteImage,
                  ),
                ...SystemContextMenu.getDefaultItems(editable),
              ],
      );
      if (menu.items.isEmpty) return const SizedBox.shrink();
      nativeController.showWithItems(
        menu.anchor,
        menu.items
            .map((item) => item.getData(WidgetsLocalizations.of(context)))
            .toList(),
      );
      return const SizedBox.shrink();
    }
    final items = formatting.value && canFormat
        ? [
            for (final style in formats)
              ContextMenuButtonItem(
                label: style.$1,
                onPressed: () => apply(style.$2, style.$3),
              ),
          ]
        : [
            if (defaultTargetPlatform == TargetPlatform.iOS && canFormat)
              ContextMenuButtonItem(
                label: 'Format',
                onPressed: () => formatting.value = true,
              ),
            if (defaultTargetPlatform == TargetPlatform.iOS &&
                clipboardHasImage)
              ContextMenuButtonItem(
                label: 'Paste Image',
                onPressed: onPasteImage,
              ),
            ...editable.contextMenuButtonItems,
          ];
    return AdaptiveTextSelectionToolbar.buttonItems(
      anchors: editable.contextMenuAnchors,
      buttonItems: items,
    );
  }
}
