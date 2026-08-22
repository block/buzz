import { Navigate, createFileRoute } from "@tanstack/react-router";

export const Route = createFileRoute("/repos")({
  component: () => <Navigate to="/" />,
});
