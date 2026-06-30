import { LifecycleActivity } from "./LifecycleActivity";
import type { ActivityRenderClassItemProps } from "./types";

export function PermissionActivity(props: ActivityRenderClassItemProps) {
  return <LifecycleActivity {...props} />;
}
