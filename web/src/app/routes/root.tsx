import { Outlet, createRootRoute } from "@tanstack/react-router";

export const Route = createRootRoute({
  component: RootLayout,
});

function RootLayout() {
  return (
    <div className="flex min-h-dvh flex-col">
      <main className="flex flex-1 flex-col">
        <Outlet />
      </main>
    </div>
  );
}
