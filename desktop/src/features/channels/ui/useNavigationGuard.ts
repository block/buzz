import * as React from "react";

import { registerNavigationGuard } from "@/app/navigation/navigationGuard";

export function useNavigationGuard(requireThreadEditResolution: () => boolean) {
  React.useLayoutEffect(
    () => registerNavigationGuard(() => requireThreadEditResolution()),
    [requireThreadEditResolution],
  );
}
