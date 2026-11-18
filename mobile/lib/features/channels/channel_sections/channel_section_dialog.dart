import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

import '../../../shared/emoji/emoji_picker.dart';
import '../../../shared/theme/buzz_icons.dart';
import '../../../shared/widgets/modal_presentation.dart';
import 'channel_section_emoji.dart';
import 'channel_sections_storage.dart';

/// Edits a section name and optional emoji without saving on dismissal.
Future<({String name, String? icon})?> showChannelSectionDialog(
  BuildContext context, {
  ChannelSection? section,
}) => showBuzzDialog<({String name, String? icon})>(
  context: context,
  builder: (_) => _SectionDialog(section: section),
);

class _SectionDialog extends HookWidget {
  const _SectionDialog({required this.section});

  final ChannelSection? section;

  @override
  Widget build(BuildContext context) {
    final name = useTextEditingController(text: section?.name ?? '');
    final icon = useState(section?.icon);
    final placeholder = const Icon(BuzzIcons.smilePlus);

    void confirm() {
      final trimmed = name.text.trim();
      if (trimmed.isNotEmpty) {
        Navigator.of(context).pop((name: trimmed, icon: icon.value));
      }
    }

    return AlertDialog(
      title: Text(section == null ? 'New Section' : 'Rename Section'),
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
        TextButton(
          onPressed: confirm,
          child: Text(section == null ? 'Create' : 'Rename'),
        ),
      ],
    );
  }
}
