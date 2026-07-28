import * as React from "react";
import { CalendarCheck, ChartGantt, Plus } from "lucide-react";
import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useIdentityQuery } from "@/shared/api/hooks";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { usePlanMutations, usePlansQuery } from "../hooks";
import { parsePlanningProject } from "../domain/contracts";

function today() {
  return new Date().toISOString().slice(0, 10);
}
function timestamp() {
  return new Date().toISOString();
}

export function PlansScreen() {
  const identity = useIdentityQuery();
  const plans = usePlansQuery(identity.data?.pubkey);
  const mutations = usePlanMutations(identity.data?.pubkey ?? "");
  const { goPlan } = useAppNavigation();
  const [open, setOpen] = React.useState(false);
  const [title, setTitle] = React.useState("");
  const [purpose, setPurpose] = React.useState("");
  const [owner, setOwner] = React.useState("Operations Officer");
  const [missionReadyDate, setMissionReadyDate] = React.useState(today());
  async function createProject() {
    const now = timestamp();
    const project = parsePlanningProject({
      schemaVersion: 1,
      id: crypto.randomUUID(),
      title,
      purpose,
      missionReadyDate,
      status: "active",
      progressPercent: 0,
      owner,
      linkedActivityIds: [],
      assumptions: [],
      createdAt: now,
      updatedAt: now,
    });
    await mutations.project.mutateAsync(project);
    setOpen(false);
    await goPlan(project.id);
  }
  return (
    <main
      className="min-h-0 flex-1 overflow-auto p-6"
      data-testid="plans-screen"
    >
      <div className="mx-auto max-w-6xl">
        <header className="flex items-start justify-between gap-4">
          <div>
            <p className="text-xs uppercase tracking-[0.18em] text-muted-foreground">
              Command planning
            </p>
            <h1 className="text-2xl font-semibold">Plans</h1>
            <p className="mt-1 max-w-2xl text-sm text-muted-foreground">
              Deployment work breakdown, critical path, readiness constraints,
              and mission-ready milestones.
            </p>
          </div>
          <button
            className="rounded bg-primary px-3 py-2 text-sm text-primary-foreground"
            onClick={() => setOpen(true)}
            type="button"
          >
            <Plus className="mr-1 inline h-4 w-4" />
            New Plan
          </button>
        </header>
        {plans.isLoading ? (
          <p className="mt-8 text-sm text-muted-foreground">Loading plans…</p>
        ) : plans.data?.projects.length ? (
          <div className="mt-6 grid gap-3 md:grid-cols-2">
            {plans.data.projects.map((project) => {
              const tasks = plans.data.tasks.filter(
                (task) => task.projectId === project.id,
              );
              const constraints = plans.data.constraints.filter(
                (constraint) =>
                  constraint.projectId === project.id &&
                  constraint.status !== "resolved",
              );
              return (
                <button
                  className="rounded-lg border bg-card p-4 text-left shadow-sm transition hover:border-primary/50"
                  key={project.id}
                  onClick={() => void goPlan(project.id)}
                  type="button"
                >
                  <div className="flex items-start justify-between gap-3">
                    <div>
                      <strong className="text-base">{project.title}</strong>
                      <p className="mt-1 line-clamp-2 text-sm text-muted-foreground">
                        {project.purpose}
                      </p>
                    </div>
                    <ChartGantt className="h-5 w-5 text-primary" />
                  </div>
                  <div className="mt-4 flex flex-wrap gap-3 text-xs text-muted-foreground">
                    <span>
                      <CalendarCheck className="mr-1 inline h-3.5 w-3.5" />
                      Ready {project.missionReadyDate}
                    </span>
                    <span>{tasks.length} tasks</span>
                    <span>{constraints.length} open constraints</span>
                    <span>{project.progressPercent}% complete</span>
                  </div>
                </button>
              );
            })}
          </div>
        ) : (
          <section className="mt-8 rounded-lg border border-dashed p-10 text-center">
            <ChartGantt className="mx-auto h-8 w-8 text-muted-foreground" />
            <h2 className="mt-3 text-base font-medium">No active plans</h2>
            <p className="mt-1 text-sm text-muted-foreground">
              Create a deployment or readiness plan to start its work breakdown.
            </p>
          </section>
        )}
      </div>
      <Dialog open={open} onOpenChange={setOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>New operational plan</DialogTitle>
          </DialogHeader>
          <div className="grid gap-3">
            <label className="grid gap-1 text-sm">
              Plan title
              <input
                className="rounded border bg-background px-3 py-2"
                onChange={(event) => setTitle(event.target.value)}
                value={title}
              />
            </label>
            <label className="grid gap-1 text-sm">
              Purpose
              <textarea
                className="min-h-20 rounded border bg-background px-3 py-2"
                onChange={(event) => setPurpose(event.target.value)}
                value={purpose}
              />
            </label>
            <label className="grid gap-1 text-sm">
              Owner
              <input
                className="rounded border bg-background px-3 py-2"
                onChange={(event) => setOwner(event.target.value)}
                value={owner}
              />
            </label>
            <label className="grid gap-1 text-sm">
              Mission-ready date
              <input
                className="rounded border bg-background px-3 py-2"
                onChange={(event) => setMissionReadyDate(event.target.value)}
                type="date"
                value={missionReadyDate}
              />
            </label>
          </div>
          <div className="flex justify-end gap-2">
            <button
              className="rounded border px-3 py-2 text-sm"
              onClick={() => setOpen(false)}
              type="button"
            >
              Cancel
            </button>
            <button
              className="rounded bg-primary px-3 py-2 text-sm text-primary-foreground disabled:opacity-50"
              disabled={!title.trim() || !purpose.trim() || !owner.trim()}
              onClick={() => void createProject()}
              type="button"
            >
              Create plan
            </button>
          </div>
        </DialogContent>
      </Dialog>
    </main>
  );
}
