import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/relay/relay.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/modal_presentation.dart';
import 'ios_profile_text_editor.dart';
import 'profile_provider.dart';

/// Opens the current user's display-name editor from a profile action surface.
Future<void> showProfileDisplayNameEditor(BuildContext context) async {
  final container = ProviderScope.containerOf(context, listen: false);
  try {
    await container.read(profileProvider.future);
  } catch (_) {
    return;
  }
  if (!context.mounted) return;
  final profileState = container.read(profileProvider);
  if (!profileState.hasValue) return;
  final profile = profileState.requireValue;
  final onSave = bindProfileSaveToOpeningContext(
    container,
    container.read(profileProvider.notifier).updateDisplayName,
  );
  await _showProfileTextEditor(
    context: context,
    title: 'Display name',
    initialValue: profile?.displayName ?? '',
    hintText: 'Display name',
    onSave: onSave,
  );
}

/// Opens the current user's profile-description editor.
Future<void> showProfileDescriptionEditor(BuildContext context) async {
  final container = ProviderScope.containerOf(context, listen: false);
  try {
    await container.read(profileProvider.future);
  } catch (_) {
    return;
  }
  if (!context.mounted) return;
  final profileState = container.read(profileProvider);
  if (!profileState.hasValue) return;
  final profile = profileState.requireValue;
  final onSave = bindProfileSaveToOpeningContext(
    container,
    container.read(profileProvider.notifier).updateAbout,
  );
  await _showProfileTextEditor(
    context: context,
    title: 'Profile description',
    initialValue: profile?.about ?? '',
    hintText: 'Profile description',
    multiline: true,
    onSave: onSave,
  );
}

/// Prevents a profile draft from being published after its community changes.
Future<void> Function(String) bindProfileSaveToOpeningContext(
  ProviderContainer container,
  Future<void> Function(String value) onSave,
) {
  final openingConfig = container.read(relayConfigProvider);
  final openingPubkey = container.read(myPubkeyProvider);
  final openingSession = container.read(relaySessionProvider.notifier);
  return (value) {
    final currentConfig = container.read(relayConfigProvider);
    final isCurrent =
        currentConfig.storedOrigin == openingConfig.storedOrigin &&
        currentConfig.nsec == openingConfig.nsec &&
        container.read(myPubkeyProvider) == openingPubkey &&
        identical(
          container.read(relaySessionProvider.notifier),
          openingSession,
        );
    if (!isCurrent) throw ProfileCommunityChangedException();
    return onSave(value);
  };
}

Future<void> _showProfileTextEditor({
  required BuildContext context,
  required String title,
  required String initialValue,
  required String hintText,
  required Future<void> Function(String value) onSave,
  bool multiline = false,
}) async {
  if (defaultTargetPlatform == TargetPlatform.iOS) {
    final sheetTheme = utilitySurfaceThemeData(Theme.of(context));
    try {
      await IosProfileTextEditor.presentUntilSaved(
        title: title,
        initialValue: initialValue,
        placeholder: hintText,
        multiline: multiline,
        brightness: sheetTheme.brightness,
        pageBackgroundColor: sheetTheme.colorScheme.surface,
        containerBackgroundColor:
            sheetTheme.colorScheme.surfaceContainerHighest,
        onSave: onSave,
        shouldRetryOnError: (error) =>
            error is! ProfileCommunityChangedException,
        canPresent: () =>
            context.mounted && (ModalRoute.of(context)?.isCurrent ?? true),
        onSaveError: () {
          if (context.mounted && (ModalRoute.of(context)?.isCurrent ?? true)) {
            _showSaveError(context);
          }
        },
      );
      return;
    } on MissingPluginException {
      // Previews and older builds retain the complete Flutter fallback.
    } on PlatformException {
      // A temporary native presentation failure should not block editing.
    }
  }

  if (!context.mounted) return;
  await showBuzzModalBottomSheet<void>(
    context: context,
    title: title,
    isScrollControlled: true,
    requestFocus: true,
    builder: (_) => _ProfileTextEditSheet(
      initialValue: initialValue,
      hintText: hintText,
      multiline: multiline,
      onSave: onSave,
    ),
  );
}

void _showSaveError(BuildContext context) {
  ScaffoldMessenger.of(context).showSnackBar(
    const SnackBar(content: Text("We couldn't save this change. Try again.")),
  );
}

class _ProfileTextEditSheet extends HookWidget {
  const _ProfileTextEditSheet({
    required this.initialValue,
    required this.hintText,
    required this.multiline,
    required this.onSave,
  });

  final String initialValue;
  final String hintText;
  final bool multiline;
  final Future<void> Function(String value) onSave;

  @override
  Widget build(BuildContext context) {
    final controller = useTextEditingController(text: initialValue);
    useListenable(controller);
    final isSaving = useState(false);
    final error = useState<String?>(null);
    final hasChanges = controller.text.trim() != initialValue.trim();

    Future<void> save() async {
      if (!hasChanges || isSaving.value) return;
      isSaving.value = true;
      error.value = null;
      try {
        await onSave(controller.text);
        if (context.mounted) Navigator.of(context).pop();
      } on ProfileCommunityChangedException {
        if (context.mounted) Navigator.of(context).pop();
      } catch (_) {
        error.value = "We couldn't save this change. Try again.";
      } finally {
        if (context.mounted) isSaving.value = false;
      }
    }

    return SafeArea(
      top: false,
      child: Padding(
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
              Text(
                error.value!,
                style: context.textTheme.bodySmall?.copyWith(
                  color: context.colors.error,
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
    );
  }
}
