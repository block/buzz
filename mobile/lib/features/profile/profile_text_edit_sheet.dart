import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../../shared/theme/theme.dart';
import '../../shared/widgets/buzz_sheet_header.dart';
import '../../shared/widgets/ios_glass_navigation_button.dart';
import 'profile_provider.dart';

/// Flutter fallback sheet for editing one profile text field.
class ProfileTextEditSheet extends HookWidget {
  /// Creates a profile text editor.
  const ProfileTextEditSheet({
    super.key,
    required this.title,
    required this.initialValue,
    required this.hintText,
    required this.multiline,
    required this.onSave,
  });

  /// The heading displayed in the sheet header.
  final String title;

  /// The value loaded into the text field when the sheet opens.
  final String initialValue;

  /// Placeholder text displayed while the text field is empty.
  final String hintText;

  /// Whether the text field accepts and displays multiple lines.
  final bool multiline;

  /// Persists the submitted value before the sheet closes.
  final Future<void> Function(String value) onSave;

  @override
  Widget build(BuildContext context) {
    final controller = useTextEditingController(text: initialValue);
    useListenable(controller);
    final isSaving = useState(false);
    final error = useState<String?>(null);
    final hasChanges = controller.text.trim() != initialValue.trim();

    Future<void> closeAfterSave() async {
      isSaving.value = false;
      await WidgetsBinding.instance.endOfFrame;
      if (context.mounted) Navigator.of(context).pop();
    }

    Future<void> save() async {
      if (!hasChanges || isSaving.value) return;
      isSaving.value = true;
      error.value = null;
      try {
        await onSave(controller.text);
        if (context.mounted) await closeAfterSave();
      } on ProfileCommunityChangedException {
        if (context.mounted) await closeAfterSave();
      } catch (_) {
        if (context.mounted) {
          error.value = "We couldn't save this change. Try again.";
        }
      } finally {
        if (context.mounted) isSaving.value = false;
      }
    }

    final closeButton = Theme.of(context).platform == TargetPlatform.iOS
        ? IosGlassNavigationButton(
            key: const ValueKey('profile-field-close'),
            icon: IosGlassNavigationIcon.close,
            semanticLabel: 'Close sheet',
            onPressed: isSaving.value
                ? null
                : () => Navigator.of(context).pop(),
            width: 44,
            height: 44,
            foregroundColor: context.colors.primary,
          )
        : SizedBox.square(
            dimension: 44,
            child: IconButton(
              key: const ValueKey('profile-field-close'),
              tooltip: 'Close sheet',
              onPressed: isSaving.value
                  ? null
                  : () => Navigator.of(context).pop(),
              style: IconButton.styleFrom(
                padding: EdgeInsets.zero,
                backgroundColor: context.colors.surfaceContainerHighest,
                foregroundColor: context.colors.primary,
                shape: RoundedRectangleBorder(
                  borderRadius: BorderRadius.circular(Radii.dialog),
                ),
              ),
              icon: const Icon(LucideIcons.x, size: 22),
            ),
          );

    return PopScope<void>(
      canPop: !isSaving.value,
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          BuzzSheetHeader(title: title, trailing: closeButton),
          Flexible(
            child: SafeArea(
              top: false,
              child: SingleChildScrollView(
                key: const ValueKey('profile-field-scroll-view'),
                padding: EdgeInsets.fromLTRB(
                  Grid.gutter,
                  Grid.xxs,
                  Grid.gutter,
                  MediaQuery.viewInsetsOf(context).bottom + Grid.xs,
                ),
                child: Column(
                  mainAxisSize: MainAxisSize.min,
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: [
                    TextField(
                      key: const ValueKey('profile-field-input'),
                      controller: controller,
                      autofocus: true,
                      enabled: !isSaving.value,
                      minLines: multiline ? 4 : 1,
                      maxLines: multiline ? 6 : 1,
                      textCapitalization: TextCapitalization.sentences,
                      textInputAction: multiline
                          ? TextInputAction.newline
                          : TextInputAction.done,
                      onSubmitted: multiline ? null : (_) => unawaited(save()),
                      decoration: InputDecoration(hintText: hintText),
                    ),
                    if (error.value != null) ...[
                      const SizedBox(height: Grid.xxs),
                      Semantics(
                        liveRegion: true,
                        child: Text(
                          error.value!,
                          style: context.textTheme.bodySmall?.copyWith(
                            color: context.colors.error,
                          ),
                        ),
                      ),
                    ],
                    const SizedBox(height: Grid.xs),
                    FilledButton(
                      key: const ValueKey('profile-field-save'),
                      onPressed: hasChanges && !isSaving.value ? save : null,
                      child: Text(isSaving.value ? 'Saving…' : 'Save'),
                    ),
                  ],
                ),
              ),
            ),
          ),
        ],
      ),
    );
  }
}
