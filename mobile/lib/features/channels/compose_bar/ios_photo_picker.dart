part of '../compose_bar.dart';

const _inlinePhotoPickerViewType = 'buzz/inline_photo_picker';
const _inlinePhotoPickerSupportChannel = MethodChannel(
  'buzz/inline_photo_picker',
);

class _IOSInlinePhotoPicker extends HookWidget {
  final VoidCallback onBack;
  final Future<List<XFile>> Function() onPickAllPhotos;
  final Future<void> Function(List<XFile> photos) onChoosePhotos;
  final Widget fallback;

  const _IOSInlinePhotoPicker({
    required this.onBack,
    required this.onPickAllPhotos,
    required this.onChoosePhotos,
    required this.fallback,
  });

  @override
  Widget build(BuildContext context) {
    final supportFuture = useMemoized(
      () async =>
          await _inlinePhotoPickerSupportChannel.invokeMethod<bool>(
            'isSupported',
          ) ??
          false,
    );
    final support = useFuture(supportFuture);
    final pickerChannel = useState<MethodChannel?>(null);
    final selectedCount = useState(0);
    final selectedPaths = useState<List<String>>(const []);
    final isPreparingSelection = useState(false);
    final isProcessing = useState(false);

    useEffect(() {
      final channel = pickerChannel.value;
      if (channel == null) return null;

      channel.setMethodCallHandler((call) async {
        switch (call.method) {
          case 'selectionCountChanged':
            selectedCount.value = call.arguments as int? ?? 0;
            selectedPaths.value = const [];
            isPreparingSelection.value = selectedCount.value > 0;
          case 'selectionDidChange':
            final paths = (call.arguments as List<Object?>? ?? const [])
                .whereType<String>()
                .toList();
            selectedPaths.value = paths;
            isPreparingSelection.value = false;
          case 'didFail':
            isPreparingSelection.value = false;
            if (!context.mounted) return;
            ScaffoldMessenger.maybeOf(context)?.showSnackBar(
              SnackBar(
                content: Text(
                  call.arguments as String? ??
                      'Unable to prepare the selected photos.',
                ),
              ),
            );
        }
      });
      return () => channel.setMethodCallHandler(null);
    }, [pickerChannel.value]);

    final canSelect =
        selectedCount.value > 0 &&
        selectedPaths.value.length == selectedCount.value &&
        !isPreparingSelection.value &&
        !isProcessing.value;

    Future<void> submitSelection() async {
      if (!canSelect) return;
      isProcessing.value = true;
      try {
        final paths = List<String>.from(selectedPaths.value);
        final claimed =
            await pickerChannel.value?.invokeMethod<bool>(
              'claimSelection',
              paths,
            ) ??
            false;
        if (!claimed) return;
        final photos = [for (final path in paths) XFile(path)];
        await processTemporaryImages(photos, onChoosePhotos);
      } finally {
        if (context.mounted) isProcessing.value = false;
      }
    }

    Future<void> openAllPhotos() async {
      if (isPreparingSelection.value || isProcessing.value) return;
      isProcessing.value = true;
      try {
        final photos = await onPickAllPhotos();
        if (photos.isNotEmpty) {
          await onChoosePhotos(photos);
        }
      } catch (_) {
        if (context.mounted) {
          ScaffoldMessenger.maybeOf(context)?.showSnackBar(
            const SnackBar(content: Text('Unable to open your photo library.')),
          );
        }
      } finally {
        if (context.mounted) isProcessing.value = false;
      }
    }

    if (support.connectionState != ConnectionState.done) {
      return const _NativePhotoPickerLoading();
    }
    if (support.hasError || support.data != true) return fallback;

    return SizedBox(
      key: const ValueKey('ios-inline-photo-picker'),
      height: _attachmentExpandedHeight,
      width: double.infinity,
      child: Stack(
        fit: StackFit.expand,
        children: [
          UiKitView(
            viewType: _inlinePhotoPickerViewType,
            creationParamsCodec: const StandardMessageCodec(),
            onPlatformViewCreated: (viewId) {
              pickerChannel.value = MethodChannel(
                'buzz/inline_photo_picker/$viewId',
              );
            },
          ),
          PositionedDirectional(
            start: Grid.twelve,
            bottom: Grid.twelve,
            child: SafeArea(
              top: false,
              child: DecoratedBox(
                decoration: BoxDecoration(
                  color: Colors.black.withValues(alpha: 0.62),
                  shape: BoxShape.circle,
                  border: Border.all(
                    color: Colors.white.withValues(alpha: 0.28),
                  ),
                ),
                child: IconButton(
                  key: const ValueKey('ios-inline-photo-picker-back'),
                  onPressed: isProcessing.value ? null : onBack,
                  tooltip: 'Back to attachment options',
                  icon: const Icon(
                    LucideIcons.chevronLeft,
                    color: Colors.white,
                  ),
                ),
              ),
            ),
          ),
          PositionedDirectional(
            end: Grid.twelve,
            bottom: Grid.twelve,
            child: SafeArea(
              top: false,
              child: FilledButton(
                key: const ValueKey('ios-inline-photo-picker-select'),
                onPressed: canSelect
                    ? submitSelection
                    : selectedCount.value == 0 &&
                          !isPreparingSelection.value &&
                          !isProcessing.value
                    ? openAllPhotos
                    : null,
                style: FilledButton.styleFrom(
                  backgroundColor: Colors.black.withValues(alpha: 0.76),
                  disabledBackgroundColor: Colors.black.withValues(alpha: 0.32),
                  foregroundColor: Colors.white,
                  disabledForegroundColor: Colors.white70,
                  minimumSize: const Size(0, 48),
                  padding: const EdgeInsets.symmetric(horizontal: Grid.twelve),
                  shape: const StadiumBorder(),
                  side: BorderSide(color: Colors.white.withValues(alpha: 0.28)),
                ),
                child: isPreparingSelection.value
                    ? const SizedBox.square(
                        dimension: 20,
                        child: CircularProgressIndicator(
                          strokeWidth: 2,
                          color: Colors.white,
                        ),
                      )
                    : Text(
                        selectedCount.value == 0
                            ? 'All Photos'
                            : 'Add ${selectedCount.value} '
                                  '${selectedCount.value == 1 ? 'photo' : 'photos'}',
                      ),
              ),
            ),
          ),
          if (isProcessing.value)
            const ColoredBox(
              color: Color.fromRGBO(0, 0, 0, 0.28),
              child: Center(
                child: CircularProgressIndicator(
                  strokeWidth: 3,
                  color: Colors.white,
                ),
              ),
            ),
        ],
      ),
    );
  }
}

class _NativePhotoPickerLoading extends StatelessWidget {
  const _NativePhotoPickerLoading();

  @override
  Widget build(BuildContext context) {
    return SizedBox(
      key: const ValueKey('ios-inline-photo-picker-loading'),
      height: _attachmentExpandedHeight,
      width: double.infinity,
      child: const Center(child: CircularProgressIndicator(strokeWidth: 3)),
    );
  }
}
