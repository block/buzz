import 'package:flutter/services.dart';

/// Opens the native iOS form used to edit a single profile text field.
class IosProfileTextEditor {
  IosProfileTextEditor._();

  static const _channel = MethodChannel('buzz/profile_text_editor');

  static Future<String?> present({
    required String title,
    required String initialValue,
    required String placeholder,
    required bool multiline,
  }) => _channel.invokeMethod<String>('present', {
    'title': title,
    'initialValue': initialValue,
    'placeholder': placeholder,
    'multiline': multiline,
  });

  /// Keeps the native editor's latest value available until it saves or the
  /// user cancels, so a transient publish failure never discards their text.
  static Future<void> presentUntilSaved({
    required String title,
    required String initialValue,
    required String placeholder,
    required bool multiline,
    required Future<void> Function(String value) onSave,
    required void Function() onSaveError,
  }) async {
    var draft = initialValue;
    while (true) {
      final value = await present(
        title: title,
        initialValue: draft,
        placeholder: placeholder,
        multiline: multiline,
      );
      if (value == null) return;
      try {
        await onSave(value);
        return;
      } catch (_) {
        draft = value;
        onSaveError();
      }
    }
  }
}
