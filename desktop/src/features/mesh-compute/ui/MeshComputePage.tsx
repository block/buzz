import * as React from "react";
import { BookOpen } from "lucide-react";

import { useCommunities } from "@/features/communities/useCommunities";
import { SettingsSectionHeader } from "@/features/settings/ui/SettingsSectionHeader";
import { Button } from "@/shared/ui/button";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/shared/ui/tabs";
import {
  COMMUNITY_COMPUTE_TUTORIAL_FIXTURES,
  type CommunityComputeTutorialFixtureId,
} from "../communityComputeFixtures";
import { useMeshComputeState } from "../hooks/useMeshComputeState";
import { CommunityComputeTutorialPanel } from "./CommunityComputeTutorialPanel";
import { MeshComputeCommunityView } from "./MeshComputeCommunityView";
import { MeshComputeImpactReceipt } from "./MeshComputeImpactReceipt";
import { MeshComputeSettingsCard } from "./MeshComputeSettingsCard";
import { MeshComputeShareBanner } from "./MeshComputeShareBanner";
import { MeshComputeTokenLeaderboard } from "./MeshComputeTokenLeaderboard";

export function MeshComputePage() {
  const { activeCommunity } = useCommunities();
  const mesh = useMeshComputeState();
  const [tab, setTab] = React.useState("community");
  const [tutorialOpen, setTutorialOpen] = React.useState(false);
  const [tutorialStep, setTutorialStep] = React.useState(0);
  const [fixtureId, setFixtureId] =
    React.useState<CommunityComputeTutorialFixtureId>("small-company");
  const communityName = activeCommunity?.name ?? "this community";
  const tutorialFixture = COMMUNITY_COMPUTE_TUTORIAL_FIXTURES[fixtureId];

  function toggleTutorial() {
    if (tutorialOpen) {
      setTutorialOpen(false);
      return;
    }
    setTab("community");
    setTutorialOpen(true);
    setTutorialStep(0);
    setFixtureId("small-company");
  }

  return (
    <section className="min-w-0" data-testid="settings-mesh-compute-page">
      <SettingsSectionHeader
        action={
          <Button
            aria-pressed={tutorialOpen}
            data-testid="community-compute-tutorial-button"
            onClick={toggleTutorial}
            size="sm"
            type="button"
            variant={tutorialOpen ? "secondary" : "outline"}
          >
            <BookOpen /> Tutorial
          </Button>
        }
        description={`Contribute this machine’s spare capacity to unlock more models and keep ${communityName}’s agents available when demand grows.`}
        title="Compute"
      />

      <div className="space-y-5">
        {tutorialOpen ? null : (
          <MeshComputeShareBanner communityName={communityName} mesh={mesh} />
        )}

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
            {tutorialOpen ? (
              <CommunityComputeTutorialPanel
                fixtureId={fixtureId}
                onExit={() => setTutorialOpen(false)}
                onFixtureChange={setFixtureId}
                onStepChange={setTutorialStep}
                step={tutorialStep}
              />
            ) : (
              <MeshComputeImpactReceipt mesh={mesh} />
            )}
            <MeshComputeTokenLeaderboard mesh={mesh} simulated={tutorialOpen} />
            <MeshComputeCommunityView
              communityName={
                tutorialOpen ? tutorialFixture.title : communityName
              }
              error={mesh.error}
              onRefresh={mesh.refreshSnapshot}
              simulated={tutorialOpen}
              snapshot={tutorialOpen ? tutorialFixture.snapshot : mesh.snapshot}
              tutorialInference={tutorialOpen}
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
