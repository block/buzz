import * as React from "react";

import { useCommunities } from "@/features/communities/useCommunities";
import { SettingsSectionHeader } from "@/features/settings/ui/SettingsSectionHeader";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/shared/ui/tabs";
import { useMeshComputeState } from "../hooks/useMeshComputeState";
import { MeshComputeCommunityView } from "./MeshComputeCommunityView";
import { MeshComputeSettingsCard } from "./MeshComputeSettingsCard";
import { MeshComputeShareBanner } from "./MeshComputeShareBanner";

export function MeshComputePage() {
  const { activeCommunity } = useCommunities();
  const mesh = useMeshComputeState();
  const [tab, setTab] = React.useState("community");
  const communityName = activeCommunity?.name ?? "this community";

  return (
    <section className="min-w-0" data-testid="settings-mesh-compute-page">
      <SettingsSectionHeader
        description={`Contribute this machine’s spare capacity to unlock more models and keep ${communityName}’s agents available when demand grows.`}
        title="Compute"
      />

      <div className="space-y-5">
        <MeshComputeShareBanner communityName={communityName} mesh={mesh} />

        <Tabs onValueChange={setTab} value={tab}>
          <TabsList className="grid w-full grid-cols-2 sm:w-72">
            <TabsTrigger data-testid="compute-tab-community" value="community">
              Community
            </TabsTrigger>
            <TabsTrigger data-testid="compute-tab-settings" value="settings">
              Settings
            </TabsTrigger>
          </TabsList>

          <TabsContent className="mt-5 space-y-5" value="community">
            <MeshComputeCommunityView
              communityName={communityName}
              error={mesh.error}
              onRefresh={mesh.refreshSnapshot}
              snapshot={mesh.snapshot}
            />
          </TabsContent>

          <TabsContent className="mt-8" value="settings">
            <MeshComputeSettingsCard hideHeader />
          </TabsContent>
        </Tabs>
      </div>
    </section>
  );
}
