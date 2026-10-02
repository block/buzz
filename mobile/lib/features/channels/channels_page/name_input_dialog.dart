part of '../channels_page.dart';

/// Single-field dialog for naming or renaming something (a section, a
/// community). Pops the trimmed name, or null when cancelled.
class _NameInputDialog extends HookWidget {
  final String title;
  final String confirmLabel;
  final String initialValue;

  const _NameInputDialog({
    required this.title,
    required this.confirmLabel,
    this.initialValue = '',
  });

  @override
  Widget build(BuildContext context) {
    final controller = useTextEditingController(text: initialValue);

    void confirm() {
      final name = controller.text.trim();
      if (name.isNotEmpty) Navigator.of(context).pop(name);
    }

    return AlertDialog(
      title: Text(title),
      content: TextField(
        controller: controller,
        autofocus: true,
        decoration: const InputDecoration(labelText: 'Name'),
        onSubmitted: (_) => confirm(),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: const Text('Cancel'),
        ),
        TextButton(onPressed: confirm, child: Text(confirmLabel)),
      ],
    );
  }
}
