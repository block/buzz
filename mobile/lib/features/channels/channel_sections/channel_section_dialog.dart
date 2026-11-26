part of '../channel_actions_sheet.dart';

/// Collects a name and optional emoji for a new section.
Future<({String name, String? icon})?> showChannelSectionDialog(
  BuildContext context, {
  required bool Function() inScope,
}) => showBuzzDialog<({String name, String? icon})>(
  context: context,
  builder: (_) => _SectionDialog(inScope: inScope),
);

class _SectionDialog extends HookConsumerWidget {
  const _SectionDialog({required this.inScope});

  final bool Function() inScope;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final name = useTextEditingController();
    final icon = useState<String?>(null);
    final placeholder = const Icon(BuzzIcons.smilePlus);
    ref.listen(relayConfigProvider, (_, _) {
      if (!inScope()) _closeOwnRoute(context);
    });
    // A switch before the first frame found no dialog to close.
    useEffect(() {
      if (!inScope()) {
        WidgetsBinding.instance.addPostFrameCallback(
          (_) => _closeOwnRoute(context),
        );
      }
      return null;
    }, const []);

    void confirm() {
      final trimmed = name.text.trim();
      if (trimmed.isNotEmpty && inScope()) {
        Navigator.of(context).pop((name: trimmed, icon: icon.value));
      }
    }

    return AlertDialog(
      title: const Text('New Section'),
      scrollable: true,
      content: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Row(
            children: [
              Expanded(
                child: TextField(
                  controller: name,
                  autofocus: true,
                  decoration: const InputDecoration(labelText: 'Name'),
                  onSubmitted: (_) => confirm(),
                ),
              ),
              IconButton(
                tooltip: icon.value == null
                    ? 'Choose section emoji'
                    : 'Change section emoji',
                icon: ChannelSectionEmoji(
                  value: icon.value,
                  size: 22,
                  fallback: placeholder,
                ),
                onPressed: () {
                  FocusScope.of(context).unfocus();
                  showEmojiPicker(
                    context: context,
                    onSelect: (emoji) {
                      if (context.mounted) icon.value = emoji;
                    },
                  );
                },
              ),
            ],
          ),
          if (icon.value != null)
            TextButton(
              onPressed: () => icon.value = null,
              child: const Text('Remove emoji'),
            ),
        ],
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: const Text('Cancel'),
        ),
        FilledButton(onPressed: confirm, child: const Text('Create')),
      ],
    );
  }
}
