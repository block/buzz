import '../../shared/theme/grid.dart';

const composerHorizontalGutter = Grid.twelve;
const collapsedComposerWidthFactor = 0.85;

/// Width of the resting composer surface within its full-width dock.
double collapsedComposerWidth(double availableWidth) =>
    (availableWidth - composerHorizontalGutter * 2).clamp(
      0.0,
      double.infinity,
    ) *
    collapsedComposerWidthFactor;

/// Maximum conversation title width, relative to the resting composer.
double conversationTitleMaxWidth(double availableWidth) =>
    collapsedComposerWidth(availableWidth) * 2 / 3;
