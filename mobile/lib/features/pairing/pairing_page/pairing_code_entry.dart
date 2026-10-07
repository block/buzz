part of '../pairing_page.dart';

/// One native editing buffer keeps paste, deletion and accessibility reliable;
/// the six visual cells mirror the desktop verification input.
class _PairingCodeEntry extends HookConsumerWidget {
  const _PairingCodeEntry({
    required this.controller,
    required this.enabled,
    required this.invalid,
    required this.onChanged,
    required this.onSubmitted,
    required this.onRejectedInput,
  });

  final TextEditingController controller;
  final bool enabled;
  final bool invalid;
  final ValueChanged<String> onChanged;
  final VoidCallback onSubmitted;
  final VoidCallback onRejectedInput;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final focus = useFocusNode();
    useListenable(focus);
    final editing = useValueListenable(controller);
    final reducedMotion = MediaQuery.disableAnimationsOf(context);
    final activeIndex = editing.selection.isValid
        ? editing.selection.start.clamp(0, 5)
        : editing.text.length.clamp(0, 5);

    void focusCell(int index) {
      if (!enabled) return;
      focus.requestFocus();
      final offset = index.clamp(0, editing.text.length);
      controller.selection = offset < editing.text.length
          ? TextSelection(baseOffset: offset, extentOffset: offset + 1)
          : TextSelection.collapsed(offset: offset);
    }

    return Stack(
      alignment: Alignment.center,
      children: [
        // The real field remains available to assistive technology and the
        // keyboard. Visual cells handle pointer placement and display only.
        Positioned.fill(
          child: Opacity(
            opacity: 0,
            alwaysIncludeSemantics: true,
            child: TextField(
              key: const Key('pairing-code-input'),
              controller: controller,
              focusNode: focus,
              enabled: enabled,
              autofocus: true,
              keyboardType: TextInputType.number,
              textInputAction: TextInputAction.done,
              autofillHints: const [AutofillHints.oneTimeCode],
              autocorrect: false,
              enableSuggestions: false,
              inputFormatters: [
                FilteringTextInputFormatter.digitsOnly,
                TextInputFormatter.withFunction((oldValue, newValue) {
                  final limited = LengthLimitingTextInputFormatter(
                    6,
                  ).formatEditUpdate(oldValue, newValue);
                  // TextField does not call onChanged when the length limit
                  // rejects another digit. Still acknowledge that attempt.
                  if (invalid &&
                      newValue.text.length > oldValue.text.length &&
                      limited.text == oldValue.text) {
                    onRejectedInput();
                  }
                  return limited;
                }),
              ],
              decoration: const InputDecoration(
                labelText: 'Desktop code, six digits',
                border: InputBorder.none,
              ),
              onChanged: onChanged,
              onSubmitted: (_) => onSubmitted(),
            ),
          ),
        ),
        ExcludeSemantics(
          child: Row(
            children: [
              for (var index = 0; index < 6; index++) ...[
                if (index > 0)
                  SizedBox(width: index == 3 ? Grid.twelve : Grid.half),
                Expanded(
                  child: GestureDetector(
                    behavior: HitTestBehavior.opaque,
                    onTap: enabled ? () => focusCell(index) : null,
                    child: AnimatedContainer(
                      key: Key('pairing-code-cell-$index'),
                      duration: reducedMotion
                          ? Duration.zero
                          : Duration(milliseconds: invalid ? 280 : 150),
                      curve: Curves.easeOut,
                      padding: const EdgeInsets.symmetric(vertical: Grid.xs),
                      decoration: BoxDecoration(
                        color: context._onboardingInputSurface,
                        borderRadius: BorderRadius.circular(12),
                        border: Border.all(
                          color: invalid
                              ? context._onboardingErrorInk
                              : focus.hasFocus && activeIndex == index
                              ? context._onboardingInk
                              : context.colors.primary.withValues(alpha: 0.15),
                        ),
                      ),
                      child: ClipRect(
                        child: AnimatedSwitcher(
                          duration: reducedMotion
                              ? Duration.zero
                              : const Duration(milliseconds: 150),
                          switchInCurve: Curves.easeOut,
                          switchOutCurve: Curves.easeIn,
                          transitionBuilder: (child, animation) =>
                              FadeTransition(
                                opacity: animation,
                                child: AnimatedBuilder(
                                  animation: animation,
                                  builder: (context, child) =>
                                      Transform.translate(
                                        offset: Offset(
                                          0,
                                          reducedMotion
                                              ? 0
                                              : 8 * (1 - animation.value),
                                        ),
                                        child: child,
                                      ),
                                  child: child,
                                ),
                              ),
                          child: Text(
                            index < editing.text.length
                                ? editing.text[index]
                                : ' ',
                            key: ValueKey(
                              index < editing.text.length
                                  ? editing.text[index]
                                  : '',
                            ),
                            textAlign: TextAlign.center,
                            style: context.textTheme.displaySmall?.copyWith(
                              fontWeight: FontWeight.w600,
                              color: context._onboardingInk,
                            ),
                          ),
                        ),
                      ),
                    ),
                  ),
                ),
              ],
            ],
          ),
        ),
      ],
    );
  }
}
