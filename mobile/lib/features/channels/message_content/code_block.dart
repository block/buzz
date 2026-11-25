part of '../message_content.dart';

const _codeCopyButtonSize = 28.0;

class _MessageCodeBlock extends HookWidget {
  final String name;
  final String code;

  /// Whether the block sits inside a message bubble.
  final bool bubble;

  /// Text color on an accent bubble; null keeps the neutral highlighted card.
  final Color? ink;

  /// Code size, matching inline code in the same body text.
  final double fontSize;

  const _MessageCodeBlock({
    required this.name,
    required this.code,
    required this.fontSize,
    this.bubble = false,
    this.ink,
  });

  @override
  Widget build(BuildContext context) {
    final isCopied = useState(false);

    Future<void> handleCopy() async {
      await copyToClipboard(context, code, message: 'Code copied');
      if (!context.mounted) return;
      isCopied.value = true;
      Future.delayed(const Duration(seconds: 2), () {
        if (context.mounted) isCopied.value = false;
      });
    }

    final ink = this.ink;
    // A fence's closing newline would otherwise render as a blank last line.
    final shownCode = code.trimRight();
    final codeBaseStyle = TextStyle(
      fontFamily: CodeStyle.fontFamily,
      fontSize: fontSize,
      height: CodeStyle.lineHeight,
      color: ink ?? context.colors.onSurface,
    );
    final isDark = context.theme.brightness == Brightness.dark;
    final codeTheme = isDark ? highlightDarkTheme : highlightLightTheme;
    // Syntax colors are tuned for neutral cards and lose contrast on accents.
    final codeSpans = useMemoized(
      () => ink == null
          ? highlightCode(shownCode, name, codeTheme, codeBaseStyle)
          : [TextSpan(text: shownCode)],
      [code, name, isDark, ink, fontSize],
    );
    // Opaque, so the copy button can share it and hide code scrolled beneath.
    // An inked block sits on the outgoing fill, so its tint is blended there.
    final surface = ink == null
        ? CodeStyle.background(context.colors)
        : Color.alphaBlend(
            chatCodeTint(ink, 0.14),
            ChatColors.of(context).outgoing,
          );
    return Container(
      margin: bubble
          ? const EdgeInsets.symmetric(
              vertical:
                  messageBubbleHorizontalInset - messageBubbleVerticalInset,
            )
          : const EdgeInsets.only(top: Grid.half),
      decoration: BoxDecoration(
        color: surface,
        borderRadius: BorderRadius.circular(
          bubble ? Radii.container - messageBubbleHorizontalInset : 12,
        ),
        border: Border.all(
          color:
              ink?.withValues(alpha: 0.24) ?? CodeStyle.border(context.colors),
        ),
      ),
      child: Stack(
        children: [
          // As on desktop, the code scrolls under the copy button rather than
          // beside it, and its end padding keeps every line end reachable.
          Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            mainAxisSize: MainAxisSize.min,
            children: [
              if (name.isNotEmpty)
                Padding(
                  padding: const EdgeInsets.fromLTRB(
                    Grid.twelve,
                    Grid.twelve,
                    Grid.twelve + _codeCopyButtonSize,
                    Grid.half,
                  ),
                  child: Text(
                    name,
                    style: context.textTheme.labelSmall?.copyWith(
                      color: ink ?? context.colors.onSurfaceVariant,
                    ),
                  ),
                ),
              SingleChildScrollView(
                scrollDirection: Axis.horizontal,
                padding: EdgeInsets.fromLTRB(
                  Grid.twelve,
                  name.isEmpty ? Grid.twelve : 0,
                  Grid.twelve + _codeCopyButtonSize,
                  Grid.twelve,
                ),
                child: RichText(
                  softWrap: false,
                  text: TextSpan(style: codeBaseStyle, children: codeSpans),
                ),
              ),
            ],
          ),
          Positioned(
            top: Grid.half + Grid.quarter,
            right: Grid.half + Grid.quarter,
            child: SizedBox(
              width: _codeCopyButtonSize,
              height: _codeCopyButtonSize,
              child: IconButton(
                onPressed: handleCopy,
                padding: EdgeInsets.zero,
                visualDensity: VisualDensity.compact,
                style: IconButton.styleFrom(
                  tapTargetSize: MaterialTapTargetSize.shrinkWrap,
                  backgroundColor: surface,
                ),
                icon: Icon(
                  isCopied.value ? BuzzIcons.check : BuzzIcons.copy,
                  size: 14,
                  color:
                      ink ??
                      (isCopied.value
                          ? context.colors.primary
                          : context.colors.onSurfaceVariant),
                ),
              ),
            ),
          ),
        ],
      ),
    );
  }
}
