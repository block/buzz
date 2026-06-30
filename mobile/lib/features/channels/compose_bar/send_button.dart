part of '../compose_bar.dart';

class _SendButton extends StatelessWidget {
  final bool isSending;
  final bool isDisabled;
  final VoidCallback onTap;

  const _SendButton({
    required this.isSending,
    required this.onTap,
    this.isDisabled = false,
  });

  @override
  Widget build(BuildContext context) {
    return SizedBox(
      width: 36,
      height: 36,
      child: IconButton(
        onPressed: (isSending || isDisabled) ? null : onTap,
        style: IconButton.styleFrom(
          backgroundColor: context.colors.primary,
          disabledBackgroundColor: context.colors.primary.withValues(
            alpha: 0.5,
          ),
          shape: RoundedRectangleBorder(
            borderRadius: BorderRadius.circular(Radii.md),
          ),
        ),
        padding: EdgeInsets.zero,
        icon: isSending
            ? SizedBox(
                width: 16,
                height: 16,
                child: CircularProgressIndicator(
                  strokeWidth: 2,
                  color: context.colors.onPrimary,
                ),
              )
            : Icon(
                LucideIcons.sendHorizontal,
                size: 18,
                color: context.colors.onPrimary,
              ),
      ),
    );
  }
}

String _formatUploadError(Object error) {
  return error.toString().replaceFirst('Exception: ', '');
}
