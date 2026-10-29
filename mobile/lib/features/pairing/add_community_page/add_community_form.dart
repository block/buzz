part of '../add_community_page.dart';

class _AddCommunityForm extends HookWidget {
  const _AddCommunityForm({
    super.key,
    required this.onBack,
    required this.onSubmitPairing,
    required this.busy,
    required this.canSubmit,
    this.errorMessage,
    this.onInputChanged,
  });
  final VoidCallback onBack;
  final ValueChanged<String> onSubmitPairing;
  final bool busy;
  final bool canSubmit;
  final String? errorMessage;
  final VoidCallback? onInputChanged;

  @override
  Widget build(BuildContext context) {
    final controller = useTextEditingController();
    final error = useState<String?>(null);
    void submit() {
      if (busy || !canSubmit) return;
      final value = controller.text.trim();
      if (value.isEmpty) {
        error.value = 'Enter a pairing code.';
        return;
      }
      FocusManager.instance.primaryFocus?.unfocus();
      onSubmitPairing(value);
    }

    return OnboardingStepPage(
      title: 'Use a pairing code',
      subtitle: 'Paste the pairing code from your desktop app.',
      onBack: onBack,
      content: TextField(
        key: const Key('community-pairing-input'),
        controller: controller,
        enabled: !busy,
        autofocus: true,
        keyboardType: TextInputType.url,
        textInputAction: TextInputAction.go,
        autocorrect: false,
        enableSuggestions: false,
        decoration: InputDecoration(
          labelText: 'Pairing code',
          hintText: 'nostrpair://... or buzz://...',
          errorText: error.value ?? errorMessage,
        ),
        onChanged: (_) {
          error.value = null;
          onInputChanged?.call();
        },
        onSubmitted: (_) => submit(),
      ),
      actions: [
        FilledButton(
          onPressed: busy || !canSubmit ? null : submit,
          child: Text(busy ? 'Connecting…' : 'Connect'),
        ),
      ],
    );
  }
}
