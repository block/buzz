part of '../pairing_page.dart';

/// SAS verification screen shown during NIP-AB pairing.
class _SasVerificationView extends HookConsumerWidget {
  final String sasCode;
  final Future<bool> Function(String)? verifyDesktopCode;
  final bool confirmed;
  final bool sendsIdentityToDesktop;
  final String biometricLabel;
  final String? errorMessage;
  final ValueChanged<bool> onProtectionChanged;
  final VoidCallback onConfirm;
  final VoidCallback onDeny;

  const _SasVerificationView({
    super.key,
    required this.sasCode,
    this.verifyDesktopCode,
    required this.confirmed,
    required this.sendsIdentityToDesktop,
    required this.biometricLabel,
    required this.errorMessage,
    required this.onProtectionChanged,
    required this.onConfirm,
    required this.onDeny,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final controller = useTextEditingController();
    final enteredCode = useValueListenable(controller).text;
    final mismatch = useState(false);
    final checkingCode = useState(false);
    final rejectedCode = useRef<String?>(null);
    final verificationError = useState<String?>(null);
    final shakeRevision = useState(0);
    final codeAccepted = useState(false);
    final confirmationSent = useRef(false);
    useEffect(() {
      if (!confirmed) confirmationSent.value = false;
      return null;
    }, [confirmed, errorMessage]);

    void confirm({bool? protection}) {
      if (confirmed || confirmationSent.value) return;
      confirmationSent.value = true;
      if (protection != null) onProtectionChanged(protection);
      onConfirm();
    }

    void checkCode(String value) {
      if (confirmed || codeAccepted.value || checkingCode.value) return;
      verificationError.value = null;
      if (verifyDesktopCode != null) {
        mismatch.value = false;
        if (value.length != 6) return;
        if (value == rejectedCode.value) {
          mismatch.value = true;
          shakeRevision.value++;
          unawaited(errorHaptic());
          return;
        }
        checkingCode.value = true;
        unawaited(() async {
          try {
            final accepted = await verifyDesktopCode!(value);
            if (!context.mounted) return;
            if (accepted) {
              codeAccepted.value = true;
              FocusScope.of(context).unfocus();
            } else {
              rejectedCode.value = value;
              mismatch.value = true;
              shakeRevision.value++;
              unawaited(errorHaptic());
            }
          } catch (_) {
            if (context.mounted) {
              verificationError.value = 'Couldn’t check the code. Try again.';
            }
          } finally {
            if (context.mounted) checkingCode.value = false;
          }
        }());
        return;
      }
      mismatch.value = value.length == 6 && value != sasCode;
      if (mismatch.value) {
        shakeRevision.value++;
        unawaited(errorHaptic());
      } else if (value.length == 6) {
        codeAccepted.value = true;
        FocusScope.of(context).unfocus();
        if (sendsIdentityToDesktop) confirm();
      }
    }

    useEffect(() {
      if (verifyDesktopCode == null) return null;
      var active = true;
      // Negotiation can arrive after the user typed a code under legacy SAS
      // rules. Re-evaluate that input against the source-only desktop code.
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (!active || !context.mounted) return;
        rejectedCode.value = null;
        mismatch.value = false;
        codeAccepted.value = false;
        checkCode(controller.text);
      });
      return () => active = false;
    }, [verifyDesktopCode != null]);

    final showProtection = codeAccepted.value && !sendsIdentityToDesktop;

    final verificationContent = Column(
      mainAxisSize: MainAxisSize.min,
      children: [
        if (showProtection) ...[
          Icon(
            biometricLabel == 'Use Face ID'
                ? BuzzIcons.scanFace
                : BuzzIcons.fingerprint,
            size: 64,
            color: context._onboardingInk,
          ),
          const SizedBox(height: Grid.sm),
          Text(
            'Protect your identity',
            textAlign: TextAlign.center,
            style: context.textTheme.headlineSmall?.copyWith(
              color: context._onboardingInk,
              fontWeight: FontWeight.w600,
            ),
          ),
          const SizedBox(height: Grid.xxs),
          Text(
            '$biometricLabel to confirm it’s you before sending your Buzz identity to another device.',
            textAlign: TextAlign.center,
            style: context.textTheme.bodyMedium?.copyWith(
              color: context._onboardingMutedInk,
            ),
          ),
        ] else if (codeAccepted.value) ...[
          Icon(BuzzIcons.shieldCheck, size: 64, color: context._onboardingInk),
          const SizedBox(height: Grid.sm),
          Text('Code confirmed', style: context.textTheme.headlineSmall),
        ] else ...[
          Text(
            'Enter pairing code',
            textAlign: TextAlign.center,
            style: context.textTheme.headlineSmall?.copyWith(
              color: context._onboardingInk,
              fontWeight: FontWeight.w600,
              letterSpacing: -0.4,
            ),
          ),
          const SizedBox(height: Grid.md),
          _PairingErrorShake(
            revision: shakeRevision.value,
            child: _PairingCodeEntry(
              controller: controller,
              enabled: !confirmed && !checkingCode.value,
              invalid: mismatch.value,
              onChanged: checkCode,
              onSubmitted: () => checkCode(enteredCode),
              onRejectedInput: () => checkCode(controller.text),
            ),
          ),
          const SizedBox(height: Grid.xxs),
          AnimatedOpacity(
            key: const Key('pairing-code-error'),
            opacity: mismatch.value ? 1 : 0,
            duration: const Duration(milliseconds: 280),
            curve: Curves.easeOut,
            child: ExcludeSemantics(
              excluding: !mismatch.value,
              child: Semantics(
                liveRegion: true,
                child: Text(
                  'Check the desktop code and try again.',
                  textAlign: TextAlign.center,
                  style: context.textTheme.bodySmall?.copyWith(
                    color: context._onboardingInk,
                  ),
                ),
              ),
            ),
          ),
        ],
        if (errorMessage != null || verificationError.value != null) ...[
          const SizedBox(height: Grid.xs),
          Text(
            errorMessage ?? verificationError.value!,
            textAlign: TextAlign.center,
            style: context.textTheme.bodySmall?.copyWith(
              color: context._onboardingInk,
            ),
          ),
        ],
      ],
    );

    final verificationActions = confirmed
        ? Row(
            mainAxisAlignment: MainAxisAlignment.center,
            children: [
              BuzzLoadingIndicator(
                size: 24,
                color: context._onboardingInk,
                semanticLabel: 'Connecting',
              ),
              const SizedBox(width: Grid.twelve),
              Text(
                'Confirmed — waiting for desktop',
                style: context.textTheme.bodySmall?.copyWith(
                  color: context._onboardingMutedInk,
                ),
              ),
            ],
          )
        : SizedBox(
            width: double.infinity,
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                if (showProtection ||
                    (codeAccepted.value && errorMessage != null)) ...[
                  FilledButton(
                    style: context._onboardingButtonStyle,
                    onPressed: () =>
                        confirm(protection: showProtection ? true : null),
                    child: Text(showProtection ? biometricLabel : 'Try again'),
                  ),
                  const SizedBox(height: Grid.xxs),
                ],
                TextButton(
                  style: context._onboardingSecondaryButtonStyle.copyWith(
                    minimumSize: const WidgetStatePropertyAll(
                      Size.fromHeight(48),
                    ),
                  ),
                  onPressed: showProtection
                      ? () => confirm(protection: false)
                      : onDeny,
                  child: Text(showProtection ? 'Skip' : 'Cancel'),
                ),
              ],
            ),
          );

    return Column(
      children: [
        Expanded(
          child: LayoutBuilder(
            builder: (context, constraints) {
              final verticalPadding = Grid.sm * 2;
              final minimumContentHeight =
                  constraints.maxHeight > verticalPadding
                  ? constraints.maxHeight - verticalPadding
                  : 0.0;
              return SingleChildScrollView(
                padding: const EdgeInsets.symmetric(vertical: Grid.sm),
                child: ConstrainedBox(
                  constraints: BoxConstraints(minHeight: minimumContentHeight),
                  child: Center(child: verificationContent),
                ),
              );
            },
          ),
        ),
        verificationActions,
        const SizedBox(height: Grid.sm),
      ],
    );
  }
}
