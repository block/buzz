import {
  ArrowLeft,
  Check,
  Copy,
  ExternalLink,
  FolderGit2,
  GitFork,
  Users,
} from "lucide-react";
import * as React from "react";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useProjectQuery } from "@/features/projects/hooks";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { resolveUserLabel } from "@/features/profile/lib/identity";
import { isSafeUrl } from "@/shared/lib/url";
import { Button } from "@/shared/ui/button";
import { TopChromeBackdrop } from "@/shared/ui/TopChromeBackdrop";
import { UserAvatar } from "@/shared/ui/UserAvatar";

function CloneUrlRow({ url }: { url: string }) {
  const [copied, setCopied] = React.useState(false);

  const handleCopy = React.useCallback(() => {
    void navigator.clipboard.writeText(url).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 2_000);
    });
  }, [url]);

  return (
    <div className="flex items-center gap-2 rounded-lg border border-border/60 bg-muted/30 px-3 py-2">
      <GitFork className="h-4 w-4 shrink-0 text-muted-foreground" />
      <code className="min-w-0 flex-1 truncate text-xs">{url}</code>
      <Button
        className="h-6 w-6 shrink-0"
        onClick={handleCopy}
        size="icon"
        variant="ghost"
      >
        {copied ? (
          <Check className="h-4 w-4 text-green-500" />
        ) : (
          <Copy className="h-4 w-4" />
        )}
      </Button>
    </div>
  );
}

type ProjectDetailScreenProps = {
  projectId: string;
};

export function ProjectDetailScreen({ projectId }: ProjectDetailScreenProps) {
  const { goProjects } = useAppNavigation();
  const projectQuery = useProjectQuery(projectId);
  const project = projectQuery.data;

  const allPubkeys = React.useMemo(
    () =>
      project ? [project.owner, ...project.contributors].filter(Boolean) : [],
    [project],
  );
  const profilesQuery = useUsersBatchQuery(allPubkeys);
  const profiles = profilesQuery.data?.profiles;

  if (projectQuery.isLoading) {
    return null;
  }

  if (projectQuery.isError) {
    return (
      <div className="flex flex-1 flex-col items-center justify-center gap-3 px-4 py-16 text-center">
        <FolderGit2 className="h-10 w-10 text-muted-foreground/40" />
        <p className="text-sm text-red-400">Failed to load project</p>
        <div className="flex items-center gap-2">
          <Button
            onClick={() => void projectQuery.refetch()}
            size="sm"
            variant="outline"
          >
            Retry
          </Button>
          <Button
            onClick={() => {
              void goProjects();
            }}
            size="sm"
            variant="ghost"
          >
            <ArrowLeft className="mr-1.5 h-4 w-4" />
            Back to Projects
          </Button>
        </div>
      </div>
    );
  }

  if (!project) {
    return (
      <div className="flex flex-1 flex-col items-center justify-center gap-3 px-4 py-16 text-center">
        <FolderGit2 className="h-10 w-10 text-muted-foreground/40" />
        <p className="text-sm text-muted-foreground">
          This project could not be found.
        </p>
        <Button
          onClick={() => {
            void goProjects();
          }}
          size="sm"
          variant="outline"
        >
          <ArrowLeft className="mr-1.5 h-4 w-4" />
          Back to Projects
        </Button>
      </div>
    );
  }

  const createdDate = new Date(project.createdAt * 1_000).toLocaleDateString(
    undefined,
    { year: "numeric", month: "long", day: "numeric" },
  );

  return (
    <div className="relative flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
      <TopChromeBackdrop />
      <div className="flex min-h-0 min-w-0 flex-1 flex-col overflow-y-auto p-4 pt-14">
        <div className="mb-4">
          <Button
            className="gap-1.5 text-muted-foreground"
            onClick={() => {
              void goProjects();
            }}
            size="sm"
            variant="ghost"
          >
            <ArrowLeft className="h-4 w-4" />
            Back to Projects
          </Button>
        </div>

        <div className="mx-auto w-full max-w-2xl space-y-6">
          <section className="space-y-2">
            <div className="flex items-center gap-2">
              <FolderGit2 className="h-4 w-4 text-muted-foreground" />
              <h2 className="text-lg font-semibold">{project.name}</h2>
            </div>
            {project.description ? (
              <p className="text-sm text-muted-foreground">
                {project.description}
              </p>
            ) : null}
          </section>

          {project.cloneUrls.length > 0 ? (
            <section className="space-y-2">
              <h3 className="text-xs font-medium uppercase tracking-wider text-muted-foreground">
                Clone
              </h3>
              <div className="space-y-1.5">
                {project.cloneUrls.map((url) => (
                  <CloneUrlRow key={url} url={url} />
                ))}
              </div>
            </section>
          ) : null}

          {project.webUrl && isSafeUrl(project.webUrl) ? (
            <section className="space-y-2">
              <h3 className="text-xs font-medium uppercase tracking-wider text-muted-foreground">
                Web
              </h3>
              <a
                className="inline-flex items-center gap-1.5 text-sm text-primary hover:underline"
                href={project.webUrl}
                rel="noopener noreferrer"
                target="_blank"
              >
                <ExternalLink className="h-4 w-4" />
                {project.webUrl}
              </a>
            </section>
          ) : null}

          {project.contributors.length > 0 ? (
            <section className="space-y-2">
              <h3 className="text-xs font-medium uppercase tracking-wider text-muted-foreground">
                <span className="flex items-center gap-1.5">
                  <Users className="h-4 w-4" />
                  Contributors ({project.contributors.length})
                </span>
              </h3>
              <div className="space-y-1.5">
                {project.contributors.map((pubkey) => {
                  const label = resolveUserLabel({ pubkey, profiles });
                  const avatarUrl =
                    profiles?.[pubkey.toLowerCase()]?.avatarUrl ?? null;
                  return (
                    <div
                      className="flex items-center gap-2 rounded-md bg-muted/30 px-3 py-1.5"
                      key={pubkey}
                    >
                      <UserAvatar
                        avatarUrl={avatarUrl}
                        displayName={label}
                        size="xs"
                      />
                      <span className="truncate text-sm text-muted-foreground">
                        {label}
                      </span>
                    </div>
                  );
                })}
              </div>
            </section>
          ) : null}

          <section className="space-y-2">
            <h3 className="text-xs font-medium uppercase tracking-wider text-muted-foreground">
              Details
            </h3>
            <div className="space-y-1 text-sm text-muted-foreground">
              <p>Created: {createdDate}</p>
              <p className="truncate">
                Owner: {resolveUserLabel({ pubkey: project.owner, profiles })}
              </p>
            </div>
          </section>
        </div>
      </div>
    </div>
  );
}
