import * as React from "react";

import {
  readFibreSort,
  resolveFibreSort,
  writeFibreSort,
  type FibreListTab,
  type FibreSort,
} from "@/features/home/ui/fibre/fibreSort";

export function useFibreSort(tab: FibreListTab) {
  const [preference, setPreference] = React.useState<FibreSort | null>(() =>
    readFibreSort(),
  );
  const sort = resolveFibreSort(tab, preference);

  const setSort = React.useCallback((next: FibreSort) => {
    setPreference(next);
    writeFibreSort(next);
  }, []);

  return { setSort, sort };
}
