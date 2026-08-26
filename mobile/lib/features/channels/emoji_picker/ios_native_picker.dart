part of '../emoji_picker.dart';

const _nativeEmojiPickerChannel = MethodChannel('buzz/native_emoji_picker');

/// Guards the process-global native method-call handler: one native sheet may
/// own it at a time. A reentrant open would replace the handler and hijack the
/// live sheet's select/dismiss callbacks, so [_presentIosEmojiPicker] coalesces
/// reentry while a presentation is in flight.
bool _iosEmojiPickerPresenting = false;

@visibleForTesting
void resetIosEmojiPickerPresentationForTest() {
  _iosEmojiPickerPresenting = false;
}

Future<void> _presentIosEmojiPicker({
  required BuildContext context,
  required void Function(String emoji) onSelect,
  VoidCallback? onDismiss,
}) async {
  // Only one native sheet owns the handler at a time. Reject a reentrant open
  // without replacing the live sheet's callbacks, and complete the rejected
  // caller so its local picker-open lifecycle is not stranded.
  if (_iosEmojiPickerPresenting) {
    onDismiss?.call();
    return;
  }
  _iosEmojiPickerPresenting = true;

  final container = ProviderScope.containerOf(context, listen: false);
  final paletteState = container.read(customEmojiPaletteProvider);
  final List<CustomEmoji> customEmoji;
  BuildContext? loadingSheetContext;
  var leavingLoadingSheet = false;
  var loadingCancelled = false;
  Future<void>? loadingSheet;

  if (paletteState case AsyncData(:final value)) {
    customEmoji = value;
  } else {
    // Palette history can take the relay timeout to resolve. Give the tap an
    // immediate, cancellable surface instead of holding the global guard while
    // the composer appears unresponsive.
    final loadingSheetBuilt = Completer<void>();
    loadingSheet =
        showBuzzModalBottomSheet<void>(
          context: context,
          isScrollControlled: true,
          showCloseButton: false,
          builder: (sheetContext) {
            loadingSheetContext = sheetContext;
            if (!loadingSheetBuilt.isCompleted) loadingSheetBuilt.complete();
            return const SizedBox(
              key: Key('ios-emoji-picker-palette-loading'),
              height: 180,
              child: Center(
                child: Column(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    CircularProgressIndicator(),
                    SizedBox(height: Grid.sm),
                    Text('Loading emoji…'),
                  ],
                ),
              ),
            );
          },
        ).whenComplete(() {
          if (leavingLoadingSheet) return;
          loadingCancelled = true;
          _iosEmojiPickerPresenting = false;
          onDismiss?.call();
        });

    try {
      customEmoji = await container.read(customEmojiPaletteProvider.future);
    } catch (_) {
      if (loadingCancelled) return;
      await loadingSheetBuilt.future;
      if (loadingCancelled) return;
      leavingLoadingSheet = true;
      if (loadingSheetContext case final sheetContext?
          when sheetContext.mounted) {
        Navigator.of(sheetContext).pop();
      }
      await loadingSheet;
      // A palette fetch failure must not strand the composer's open state: fall
      // back to the Flutter picker, which watches the palette itself.
      _iosEmojiPickerPresenting = false;
      if (context.mounted) {
        _showFlutterEmojiPicker(
          context: context,
          onSelect: onSelect,
          onDismiss: onDismiss,
        );
      }
      return;
    }

    if (loadingCancelled) return;
    await loadingSheetBuilt.future;
    if (loadingCancelled) return;
    leavingLoadingSheet = true;
    if (loadingSheetContext case final sheetContext?
        when sheetContext.mounted) {
      Navigator.of(sheetContext).pop();
    }
    await loadingSheet;
  }
  if (!context.mounted) {
    _iosEmojiPickerPresenting = false;
    return;
  }
  final recent = container.read(recentEmojiProvider);
  final mediaAuth = container.read(mediaGetAuthServiceProvider);
  final prefs = container.read(savedPrefsProvider);
  final colors = context.colors;
  var dismissed = false;

  void finish() {
    if (dismissed) return;
    dismissed = true;
    _iosEmojiPickerPresenting = false;
    _nativeEmojiPickerChannel.setMethodCallHandler(null);
    onDismiss?.call();
  }

  _nativeEmojiPickerChannel.setMethodCallHandler((call) async {
    switch (call.method) {
      case 'mediaHeaders':
        final url = call.arguments;
        return url is String
            ? mediaAuth.headersFor(url)
            : const <String, String>{};
      case 'selected':
        final emoji = call.arguments;
        if (emoji is String && emoji.isNotEmpty) onSelect(emoji);
        return null;
      case 'dismissed':
        finish();
        return null;
      case 'skinToneChanged':
        final value = call.arguments;
        if (value is int) {
          await prefs.setInt(_emojiSkinTonePrefsKey, _validSkinTone(value));
        }
        return null;
    }
  });

  try {
    final presented = await _nativeEmojiPickerChannel.invokeMethod<bool>(
      'present',
      <String, Object>{
        'customEmoji': [
          for (final emoji in customEmoji)
            <String, String>{'shortcode': emoji.shortcode, 'url': emoji.url},
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

  if (dismissed || !context.mounted) {
    // `dismissed` means finish() already released the guard; the unmounted
    // path releases it here so a future open is not blocked.
    _iosEmojiPickerPresenting = false;
    return;
  }
  dismissed = true;
  _iosEmojiPickerPresenting = false;
  _nativeEmojiPickerChannel.setMethodCallHandler(null);
  _showFlutterEmojiPicker(
    context: context,
    onSelect: onSelect,
    onDismiss: onDismiss,
  );
}
