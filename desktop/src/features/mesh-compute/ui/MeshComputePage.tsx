import { useCommunities } from "@/features/communities/useCommunities";
import { SettingsSectionHeader } from "@/features/settings/ui/SettingsSectionHeader";
import { useMeshComputeState } from "../hooks/useMeshComputeState";
import { MeshComputeCommunityView } from "./MeshComputeCommunityView";
import { MeshComputeShareBanner } from "./MeshComputeShareBanner";

export function MeshComputePage() {
  const { activeCommunity } = useCommunities();
  const mesh = useMeshComputeState();
  const communityName = activeCommunity?.name ?? "this community";

  return (
    <section className="min-w-0" data-testid="settings-mesh-compute-page">
      <SettingsSectionHeader
        description={`Share this machine’s spare capacity with ${communityName}, then see everyone contributing to the mesh.`}
        title="Compute"
      />

      <div className="space-y-5">
        <MeshComputeShareBanner communityName={communityName} mesh={mesh} />
        <MeshComputeCommunityView
          communityName={communityName}
          error={mesh.error}
          isPreparing={mesh.pendingAction === "start"}
          onRefresh={mesh.refreshSnapshot}
          snapshot={mesh.snapshot}
        />
      </div>
    </section>
  );
}
