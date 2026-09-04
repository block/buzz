import * as React from "react";
import { detectPrefixQuery } from "@/shared/lib/detectPrefixQuery";

type EditorSnapshot = { text: string; cursor: number };
export type MentionRequest = EditorSnapshot & {
  query: string;
  startIndex: number;
  explicit: boolean;
  firstAgent: boolean;
  scope: object;
};

/** One open completion request. Closing or changing text abandons its results. */
export function useMentionQuery(
  getSnapshot: (() => EditorSnapshot) | undefined,
  scope: object,
) {
  const [request, setRequest] = React.useState<MentionRequest | null>(null);
  const current = React.useRef(request);
  const input = React.useRef<EditorSnapshot>({ text: "", cursor: 0 });
  const snapshot = React.useRef(getSnapshot);
  snapshot.current = getSnapshot;
  const searchableNamesLowerRef = React.useRef<string[]>([]);
  const publish = React.useCallback((next: MentionRequest | null) => {
    current.current = next;
    setRequest(next);
  }, []);
  const cancel = React.useCallback(() => publish(null), [publish]);
  React.useEffect(() => {
    if (current.current?.scope !== scope) cancel();
    return () => {
      current.current = null;
    };
  }, [scope, cancel]);
  const read = React.useCallback(
    () => snapshot.current?.() ?? input.current,
    [],
  );
  const prefixFor = React.useCallback(
    ({ text, cursor }: EditorSnapshot) =>
      detectPrefixQuery("@", text, cursor, searchableNamesLowerRef.current),
    [],
  );
  const update = React.useCallback(
    (text: string, cursor: number) => {
      const previous = input.current;
      input.current = { text, cursor };
      if (previous.text === text && previous.cursor === cursor) return;
      const prefix = prefixFor(input.current);
      const old = current.current;
      // Moving out of the completion (or moving in a no-trigger menu) closes it.
      if (!prefix && !(old?.explicit && previous.text !== text)) {
        cancel();
        return;
      }
      if (
        old?.scope === scope &&
        prefix?.query === old.query &&
        prefix?.startIndex === old.startIndex
      )
        return;
      publish({
        text,
        cursor,
        scope,
        query: prefix?.query ?? "",
        startIndex: prefix?.startIndex ?? cursor,
        explicit: !prefix,
        firstAgent: false,
      });
    },
    [cancel, prefixFor, publish, scope],
  );
  const open = React.useCallback(
    (cursor: number, firstAgent = false) => {
      const value = { ...read(), cursor };
      input.current = value;
      publish({
        ...value,
        query: "",
        startIndex: cursor,
        explicit: true,
        firstAgent,
        scope,
      });
    },
    [publish, read, scope],
  );
  const isCurrent = React.useCallback(() => {
    if (!request || current.current !== request || request.scope !== scope)
      return false;
    const live = read();
    if (request.explicit)
      return live.text === request.text && live.cursor === request.cursor;
    const prefix = prefixFor(live);
    return (
      prefix?.startIndex === request.startIndex &&
      prefix.query === request.query
    );
  }, [prefixFor, read, request, scope]);
  return {
    request: request?.scope === scope ? request : null,
    cancel,
    update,
    open,
    read,
    isCurrent,
    searchableNamesLowerRef,
    currentPrefix: () => prefixFor(read()),
  };
}
