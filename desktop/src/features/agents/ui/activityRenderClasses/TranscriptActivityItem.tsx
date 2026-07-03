import type { AgentActivityRenderClass } from "../agentSessionTypes";
import { LifecycleActivity } from "./LifecycleActivity";
import { MessageActivity } from "./MessageActivity";
import { PlanActivity } from "./PlanActivity";
import { RawRailActivity } from "./RawRailActivity";
import { SuppressedActivity } from "./SuppressedActivity";
import { ThoughtActivity } from "./ThoughtActivity";
import { ToolActivity } from "./ToolActivity";
import type {
  ActivityRenderClassItemProps,
  ActivityRenderClassPresenter,
} from "./types";

// Exhaustive render-class routing. Several semantic classes intentionally share
// a presenter when their row treatment is the same.
export const ACTIVITY_RENDER_CLASS_PRESENTERS = {
  message: MessageActivity,
  "relay-op": ToolActivity,
  "file-edit": ToolActivity,
  "file-read": ToolActivity,
  "skill-read": ToolActivity,
  image: ToolActivity,
  shell: ToolActivity,
  status: LifecycleActivity,
  thought: ThoughtActivity,
  plan: PlanActivity,
  permission: LifecycleActivity,
  error: LifecycleActivity,
  generic: ToolActivity,
  "raw-rail": RawRailActivity,
  suppressed: SuppressedActivity,
} satisfies Record<AgentActivityRenderClass, ActivityRenderClassPresenter>;

export function TranscriptActivityItem(props: ActivityRenderClassItemProps) {
  const Presenter = ACTIVITY_RENDER_CLASS_PRESENTERS[props.item.renderClass];
  return <Presenter {...props} />;
}
