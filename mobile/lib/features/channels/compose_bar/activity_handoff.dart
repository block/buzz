part of '../compose_bar.dart';

void _useComposerActivityFocusHandoff({
  required FocusNode focusNode,
  required ValueNotifier<bool> activityInteractionLock,
  required ValueNotifier<int> composerActivationRequests,
  required ValueNotifier<bool> isEmojiPickerOpen,
  required VoidCallback collapseComposer,
}) {
  useEffect(() {
    void coordinateFocus() {
      if (focusNode.hasFocus) {
        composerActivationRequests.value += 1;
        if (activityInteractionLock.value) {
          focusNode.unfocus(
            disposition: UnfocusDisposition.previouslyFocusedChild,
          );
        }
      } else if (!focusNode.hasFocus && !isEmojiPickerOpen.value) {
        collapseComposer();
      }
    }

    focusNode.addListener(coordinateFocus);
    WidgetsBinding.instance.addPostFrameCallback((_) => coordinateFocus());
    return () => focusNode.removeListener(coordinateFocus);
  }, [focusNode, activityInteractionLock, composerActivationRequests]);
}

bool _requestComposerHandoff(
  ValueNotifier<bool> activityInteractionLock,
  ValueNotifier<int> composerActivationRequests,
) {
  composerActivationRequests.value += 1;
  return activityInteractionLock.value;
}
