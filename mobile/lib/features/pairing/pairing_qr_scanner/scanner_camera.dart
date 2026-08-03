part of '../pairing_qr_scanner.dart';

class _QrScannerCamera extends HookWidget {
  const _QrScannerCamera({required this.onDetect, this.cameraBuilder});

  final ValueChanged<String> onDetect;
  final PairingQrScannerCameraBuilder? cameraBuilder;

  @override
  Widget build(BuildContext context) {
    final cameraError = useState<Exception?>(null);

    final testCamera = cameraBuilder;
    if (testCamera != null) {
      return testCamera(onDetect);
    }

    final error = cameraError.value;
    if (error != null) {
      final permissionDenied = error.toString().toLowerCase().contains(
        'accessdenied',
      );
      final message = permissionDenied
          ? 'Camera permission is required to scan QR codes.\n\n'
                'Please grant camera access in your device settings.'
          : 'Could not start camera: $error';
      return ColoredBox(
        color: Colors.black,
        child: Center(
          child: Padding(
            padding: const EdgeInsets.all(Grid.sm),
            child: Column(
              mainAxisSize: MainAxisSize.min,
              children: [
                Icon(
                  LucideIcons.cameraOff,
                  size: 48,
                  color: Colors.white.withValues(alpha: 0.72),
                ),
                const SizedBox(height: Grid.xs),
                Text(
                  message,
                  textAlign: TextAlign.center,
                  style: context.textTheme.bodyMedium?.copyWith(
                    color: Colors.white.withValues(alpha: 0.72),
                  ),
                ),
              ],
            ),
          ),
        ),
      );
    }

    return ReaderWidget(
      codeFormat: Format.qrCode,
      showScannerOverlay: false,
      showFlashlight: false,
      showToggleCamera: false,
      showGallery: false,
      onControllerCreated: (_, error) {
        if (error != null) {
          cameraError.value = error;
        }
      },
      onScan: (result) {
        final value = result.text;
        if (result.isValid && value != null && value.isNotEmpty) {
          onDetect(value);
        }
      },
    );
  }
}
