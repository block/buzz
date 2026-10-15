import { useProjectsQuery } from "@/features/projects/hooks";
import { isProjectHomeChannel } from "./projectHomeSelection";

export {
  findProjectHomeByChannelId,
  hasAuthoritativeHomeBinding,
  isProjectHomeChannel,
  type ProjectHomeCandidate,
} from "./projectHomeSelection";

export function useIsProjectHomeChannel(channelId: string | null | undefined) {
  const projectsQuery = useProjectsQuery();
  return isProjectHomeChannel(channelId, projectsQuery.data ?? []);
}
