import { createFileRoute } from "@tanstack/react-router";
import { AppShell } from "@/features/sessions/ui/AppShell";

export const Route = createFileRoute("/")({ component: AppShell });
