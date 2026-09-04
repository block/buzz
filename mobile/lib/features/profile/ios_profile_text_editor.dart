import 'package:flutter/services.dart';

import '../../shared/theme/theme.dart';

/// Opens the native iOS form used to edit a single profile text field.
class IosProfileTextEditor {
  IosProfileTextEditor._();

  static const _channel = MethodChannel('buzz/profile_text_editor');

  /// Presents the native editor and returns its submitted value, or null when
  /// the user cancels.
  static Future<String?> present({
    required String title,
    required String initialValue,
    required String placeholder,
    required bool multiline,
    required Brightness brightness,
    required Color pageBackgroundColor,
    required Color containerBackgroundColor,
    bool allowUnchangedSubmission = false,
  }) => _channel.invokeMethod<String>('present', {
    'title': title,
    'initialValue': initialValue,
    'placeholder': placeholder,
    'multiline': multiline,
    'brightness': brightness.name,
    'pageBackgroundArgb': pageBackgroundColor.toARGB32(),
    'containerBackgroundArgb': containerBackgroundColor.toARGB32(),
    'containerCornerRadius': Radii.container,
    'allowUnchangedSubmission': allowUnchangedSubmission,
  });

  /// Keeps the native editor's latest value available until it saves or the
  /// user cancels, so a transient publish failure never discards their text.
  static Future<void> presentUntilSaved({
    required String title,
    required String initialValue,
    required String placeholder,
    required bool multiline,
    required Brightness brightness,
    required Color pageBackgroundColor,
    required Color containerBackgroundColor,
    required Future<void> Function(String value) onSave,
    required void Function() onSaveError,
    bool Function(Object error)? shouldRetryOnError,
    bool Function()? canPresent,
  }) async {
    var draft = initialValue;
    var isRetry = false;
    while (true) {
      if (canPresent?.call() == false) return;
      final value = await present(
        title: title,
        initialValue: draft,
        placeholder: placeholder,
        multiline: multiline,
        brightness: brightness,
        pageBackgroundColor: pageBackgroundColor,
        containerBackgroundColor: containerBackgroundColor,
        allowUnchangedSubmission: isRetry,
      );
      if (value == null) return;
      try {
        await onSave(value);
        return;
      } catch (error) {
        if (shouldRetryOnError?.call(error) == false ||
            canPresent?.call() == false) {
          return;
        }
        draft = value;
        isRetry = true;
        onSaveError();
      }
    }
  }
}
