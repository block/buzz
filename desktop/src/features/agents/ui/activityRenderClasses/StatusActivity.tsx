import { LifecycleActivity } from "./LifecycleActivity";
import type { ActivityRenderClassItemProps } from "./types";

export function StatusActivity(props: ActivityRenderClassItemProps) {
  return <LifecycleActivity {...props} />;
}
