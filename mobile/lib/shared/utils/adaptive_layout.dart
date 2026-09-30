/// Minimum logical width for Buzz's persistent tablet workspace.
const double tabletWorkspaceMinWidth = 840;

/// Minimum logical shortest side for a window to be treated as tablet-class.
///
/// Requiring both dimensions keeps wide landscape phones in the compact shell
/// while still allowing iPad Split View and Stage Manager to fall back as the
/// window becomes too narrow for persistent navigation.
const double tabletWorkspaceMinShortestSide = 600;

/// Whether a window can support Buzz's persistent navigation workspace.
bool usesTabletWorkspace({required double width, required double height}) =>
    width >= tabletWorkspaceMinWidth &&
    (width < height ? width : height) >= tabletWorkspaceMinShortestSide;
