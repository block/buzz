part of '../pairing_qr_scanner.dart';

class _QrScannerCamera extends StatelessWidget {
  const _QrScannerCamera({required this.controller, required this.onDetect});

  final MobileScannerController controller;
  final ValueChanged<BarcodeCapture> onDetect;

  @override
  Widget build(BuildContext context) {
    return MobileScanner(
      controller: controller,
      fit: BoxFit.cover,
      errorBuilder: (context, error) {
        final message = switch (error.errorCode) {
          MobileScannerErrorCode.permissionDenied =>
            'Camera permission is required to scan QR codes.\n\n'
                'Please grant camera access in your device settings.',
          _ =>
            'Could not start camera: '
                "${error.errorDetails?.message ?? 'unknown error'}",
        };
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
      },
      onDetect: onDetect,
    );
  }
}
