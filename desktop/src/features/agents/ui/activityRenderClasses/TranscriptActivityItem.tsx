import type { AgentActivityRenderClass } from "../agentSessionTypes";
import { ErrorActivity } from "./ErrorActivity";
import { FileEditActivity } from "./FileEditActivity";
import { GenericActivity } from "./GenericActivity";
import { MessageActivity } from "./MessageActivity";
import { PermissionActivity } from "./PermissionActivity";
import { PlanActivity } from "./PlanActivity";
import { RawRailActivity } from "./RawRailActivity";
import { RelayOpActivity } from "./RelayOpActivity";
import { ShellActivity } from "./ShellActivity";
import { StatusActivity } from "./StatusActivity";
import { SuppressedActivity } from "./SuppressedActivity";
import { ThoughtActivity } from "./ThoughtActivity";
import type {
  ActivityRenderClassItemProps,
  ActivityRenderClassPresenter,
} from "./types";

export const ACTIVITY_RENDER_CLASS_PRESENTERS = {
  message: MessageActivity,
  "relay-op": RelayOpActivity,
  "file-edit": FileEditActivity,
  shell: ShellActivity,
  status: StatusActivity,
  thought: ThoughtActivity,
  plan: PlanActivity,
  permission: PermissionActivity,
  error: ErrorActivity,
  generic: GenericActivity,
  "raw-rail": RawRailActivity,
  suppressed: SuppressedActivity,
} satisfies Record<AgentActivityRenderClass, ActivityRenderClassPresenter>;

export function TranscriptActivityItem(props: ActivityRenderClassItemProps) {
  const Presenter = ACTIVITY_RENDER_CLASS_PRESENTERS[props.item.renderClass];
  return <Presenter {...props} />;
}
