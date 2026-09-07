import { createFileRoute } from "@tanstack/react-router";
import { ProvingGroundPage } from "@/features/proving-ground/ui/ProvingGroundPage";

export const Route = createFileRoute("/proving-ground")({
  component: ProvingGroundPage,
});
