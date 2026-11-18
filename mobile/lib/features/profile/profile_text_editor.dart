import 'package:flutter/material.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/relay/relay.dart';
import '../../shared/widgets/text_editor.dart';
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
}) => showTextEditor(
  context: context,
  title: title,
  initialValue: initialValue,
  hintText: hintText,
  onSave: onSave,
  multiline: multiline,
  keyPrefix: 'profile-field',
  shouldRetryOnError: (error) => error is! ProfileCommunityChangedException,
);
