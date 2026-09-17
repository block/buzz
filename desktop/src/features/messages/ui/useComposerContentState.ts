import * as React from "react";

export function useComposerContentState() {
  const contentRef = React.useRef("");
  const [isContentEmpty, setIsContentEmpty] = React.useState(true);
  const [composerText, setComposerText] = React.useState("");

  const setComposerContentFromText = React.useCallback((nextText: string) => {
    setComposerText(nextText);
    setIsContentEmpty((wasEmpty) => {
      const isEmpty = nextText.trim().length === 0;
      return wasEmpty === isEmpty ? wasEmpty : isEmpty;
    });
  }, []);

  const setComposerContent = React.useCallback(
    (nextContent: string) => {
      contentRef.current = nextContent;
      setComposerContentFromText(nextContent);
    },
    [setComposerContentFromText],
  );

  const syncContentRefFromEditorRef = React.useRef<() => string>(
    () => contentRef.current,
  );

  const syncComposerContentFromEditor = React.useCallback(
    () => syncContentRefFromEditorRef.current(),
    [],
  );

  return {
    contentRef,
    composerText,
    isContentEmpty,
    setComposerContent,
    setComposerContentFromText,
    syncComposerContentFromEditor,
    syncContentRefFromEditorRef,
  };
}
