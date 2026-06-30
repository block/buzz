import { LifecycleActivity } from "./LifecycleActivity";
import type { ActivityRenderClassItemProps } from "./types";

export function ErrorActivity(props: ActivityRenderClassItemProps) {
  return <LifecycleActivity {...props} />;
}
