import { useDesktopControlImports } from "@/features/agents/useDesktopControlImports";
import { useAppNavigation } from "./navigation/useAppNavigation";

/** Add local-control draft routing to the app's navigation facade. */
export function useAppNavigationWithDesktopControl() {
  const navigation = useAppNavigation();
  useDesktopControlImports(navigation.goAgents);
  return navigation;
}
