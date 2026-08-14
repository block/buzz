part of '../emoji_picker.dart';

const _nativeEmojiPickerChannel = MethodChannel('buzz/native_emoji_picker');

Future<void> _presentIosEmojiPicker({
  required BuildContext context,
  required void Function(String emoji) onSelect,
  VoidCallback? onDismiss,
}) async {
  final container = ProviderScope.containerOf(context, listen: false);
  final customEmoji = await container.read(customEmojiPaletteProvider.future);
  if (!context.mounted) return;
  final recent = container.read(recentEmojiProvider);
  final mediaAuth = container.read(mediaGetAuthServiceProvider);
  final prefs = container.read(savedPrefsProvider);
  final colors = context.colors;
  var dismissed = false;

  void finish() {
    if (dismissed) return;
    dismissed = true;
    _nativeEmojiPickerChannel.setMethodCallHandler(null);
    onDismiss?.call();
  }

  _nativeEmojiPickerChannel.setMethodCallHandler((call) async {
    switch (call.method) {
      case 'selected':
        final emoji = call.arguments;
        if (emoji is String && emoji.isNotEmpty) onSelect(emoji);
        return;
      case 'dismissed':
        finish();
        return;
      case 'skinToneChanged':
        final value = call.arguments;
        if (value is int) {
          await prefs.setInt(_emojiSkinTonePrefsKey, _validSkinTone(value));
        }
        return;
    }
  });

  try {
    final presented = await _nativeEmojiPickerChannel.invokeMethod<bool>(
      'present',
      <String, Object>{
        'customEmoji': [
          for (final emoji in customEmoji)
            <String, Object>{
              'shortcode': emoji.shortcode,
              'url': emoji.url,
              'headers': mediaAuth.headersFor(emoji.url),
            },
        ],
        'recent': [for (final entry in recent) entry.emoji],
        'skinTone': _validSkinTone(prefs.getInt(_emojiSkinTonePrefsKey)),
        'surfaceColor': colors.surface.toARGB32(),
        'controlColor': colors.surfaceContainerHighest.toARGB32(),
        'textColor': colors.onSurface.toARGB32(),
        'secondaryTextColor': colors.onSurfaceVariant.toARGB32(),
        'accentColor': colors.primary.toARGB32(),
        'dividerColor': colors.outlineVariant.toARGB32(),
        'isDark': Theme.of(context).brightness == Brightness.dark,
      },
    );
    if (presented == true) return;
  } on MissingPluginException {
    // Older builds keep the complete Flutter picker as a safe fallback.
  } on PlatformException {
    // A native presentation failure should not remove the emoji affordance.
  }

  if (dismissed || !context.mounted) return;
  dismissed = true;
  _nativeEmojiPickerChannel.setMethodCallHandler(null);
  _showFlutterEmojiPicker(
    context: context,
    onSelect: onSelect,
    onDismiss: onDismiss,
  );
}
