import { useCallback, useEffect, useRef, useState } from "react";

export interface Resource<T> {
  data?: T;
  error?: Error;
  loading: boolean;
  stale: boolean;
  refetch: () => void;
}

export function useResource<T>(
  load: () => Promise<T>,
  key: string,
): Resource<T> {
  const [data, setData] = useState<T>();
  const [error, setError] = useState<Error>();
  const [loading, setLoading] = useState(true);
  const [revision, setRevision] = useState(0);
  const loadRef = useRef(load);
  const activeRequest = useRef("");
  loadRef.current = load;
  const refetch = useCallback(() => setRevision((value) => value + 1), []);

  useEffect(() => {
    const requestId = `${key}\0${revision}`;
    activeRequest.current = requestId;
    const isCurrent = () => activeRequest.current === requestId;
    const loadCurrent = async () => {
      setLoading(true);
      setError(undefined);
      try {
        const value = await loadRef.current();
        if (isCurrent()) setData(value);
      } catch (reason) {
        if (isCurrent()) {
          setError(
            reason instanceof Error ? reason : new Error("Request failed"),
          );
        }
      } finally {
        if (isCurrent()) setLoading(false);
      }
    };
    void loadCurrent();
    return () => {
      activeRequest.current = "";
    };
  }, [key, revision]);

  return {
    data,
    error,
    loading,
    stale: loading && data !== undefined,
    refetch,
  };
}
