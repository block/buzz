import { createFileRoute } from "@tanstack/react-router";
import { JoinPage } from "@/features/join/JoinPage";

export const Route = createFileRoute("/join")({
  component: JoinPage,
});
