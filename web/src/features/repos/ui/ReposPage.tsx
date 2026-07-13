import { BookMarked, GitBranch } from "lucide-react";
import { toast } from "sonner";
import { useEffect, useMemo, useState } from "react";

import buzzAppIcon from "../../../../../desktop/public/app-icon@3x.png";
import { Input } from "@/shared/ui/input";
import { mockRepos } from "../mock-repos";
import { useRepos } from "../use-repos";
import { ConnectButton } from "./ConnectButton";
import { OrgSidebar } from "./OrgSidebar";
import { RepoListItem } from "./RepoListItem";

type SortOrder = "newest" | "oldest" | "name";

function ListItemSkeleton() {
  return (
    <div className="py-6">
      <div className="flex items-center gap-2">
        <div className="h-4 w-4 shrink-0 animate-pulse rounded bg-black/10 dark:bg-white/10" />
        <div className="h-5 w-48 animate-pulse rounded bg-black/10 dark:bg-white/10" />
        <div className="h-5 w-14 animate-pulse rounded bg-black/10 dark:bg-white/10" />
      </div>
      <div className="mt-2 h-4 w-3/4 animate-pulse rounded bg-black/10 dark:bg-white/10" />
      <div className="mt-2 flex gap-4">
        <div className="h-3 w-24 animate-pulse rounded bg-black/10 dark:bg-white/10" />
        <div className="h-3 w-20 animate-pulse rounded bg-black/10 dark:bg-white/10" />
      </div>
    </div>
  );
}

function SearchEmptyState() {
  return (
    <div className="flex flex-col items-center justify-center py-20 text-center">
      <div className="flex h-14 w-14 items-center justify-center rounded-full bg-black/5 dark:bg-white/10">
        <GitBranch className="h-7 w-7 text-black/50 dark:text-white/50" />
      </div>
      <h2 className="mt-4 text-lg font-semibold text-black dark:text-white">
        No matching repositories
      </h2>
      <p className="mt-1 max-w-sm text-sm text-black/60 dark:text-white/60">
        Try adjusting your search term.
      </p>
    </div>
  );
}

function RelayEmptyState() {
  return (
    <div className="flex flex-1 items-center justify-center bg-[#F3F3F3] px-4 py-16 text-center dark:bg-[#171717]">
      <div className="flex w-full max-w-xl flex-col items-center px-6 py-10 sm:px-12 sm:py-12">
        <div
          className="h-16 w-16 overflow-hidden bg-black"
          style={{ borderRadius: "22.37%" }}
        >
          <img alt="Buzz" className="h-full w-full" src={buzzAppIcon} />
        </div>
        <h1 className="mt-6 text-2xl font-semibold tracking-tight text-black dark:text-white">
          This relay is empty
        </h1>
        <p className="mt-2 max-w-md text-sm leading-relaxed text-black/60 dark:text-white/60">
          Repositories pushed to this relay will show up here. Open this relay
          in the Buzz desktop app to start pushing code.
        </p>
        <ConnectButton className="mt-6" />
      </div>
    </div>
  );
}

export function ReposPage() {
  const preview = import.meta.env.DEV
    ? new URLSearchParams(window.location.search).get("preview")
    : null;
  const showMockRepos = preview === "repositories";
  const showMockEmptyState = preview === "empty";
  const {
    data: fetchedRepos,
    isLoading: isLoadingRepos,
    error,
  } = useRepos({ enabled: !showMockRepos && !showMockEmptyState });
  const repos = showMockRepos
    ? mockRepos
    : showMockEmptyState
      ? []
      : fetchedRepos;
  const isLoading = preview ? false : isLoadingRepos;
  const [search, setSearch] = useState("");
  const [sort, setSort] = useState<SortOrder>("newest");

  useEffect(() => {
    if (error) {
      toast.error("Failed to load repositories", {
        description: error.message,
      });
    }
  }, [error]);

  const filteredRepos = useMemo(() => {
    if (!repos) return [];

    const term = search.toLowerCase();
    let result = repos.filter(
      (r) =>
        r.name.toLowerCase().includes(term) ||
        r.description.toLowerCase().includes(term),
    );

    switch (sort) {
      case "newest":
        result = result.sort((a, b) => b.createdAt - a.createdAt);
        break;
      case "oldest":
        result = result.sort((a, b) => a.createdAt - b.createdAt);
        break;
      case "name":
        result = result.sort((a, b) =>
          a.name.localeCompare(b.name, undefined, { sensitivity: "base" }),
        );
        break;
    }

    return result;
  }, [repos, search, sort]);

  if (isLoading) {
    return (
      <div className="flex w-full flex-1 gap-8 bg-[#F3F3F3] px-4 py-8 dark:bg-[#171717]">
        <div className="min-w-0 flex-1">
          <h2 className="mb-4 flex items-center gap-2 text-lg font-semibold text-black dark:text-white">
            <BookMarked className="h-4 w-4" /> Repositories
          </h2>
          <div className="divide-y">
            {["a", "b", "c", "d", "e"].map((key) => (
              <ListItemSkeleton key={key} />
            ))}
          </div>
        </div>
        <aside className="hidden w-72 shrink-0 lg:block" />
      </div>
    );
  }

  if (!repos || repos.length === 0) {
    return <RelayEmptyState />;
  }

  return (
    <div className="flex w-full flex-1 gap-8 bg-[#F3F3F3] px-4 py-8 dark:bg-[#171717]">
      {/* Main content */}
      <div className="min-w-0 flex-1">
        {/* Mobile-only connect button */}
        <div className="mb-4 lg:hidden">
          <ConnectButton className="w-full" />
        </div>

        <h2 className="mb-4 flex items-center gap-2 text-lg font-semibold text-black dark:text-white">
          <BookMarked className="h-4 w-4" /> Repositories
        </h2>

        {/* Search + Sort bar */}
        <div className="mb-4 flex gap-3">
          <Input
            placeholder="Find a repository..."
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            className="flex-1 border-black/10 bg-white text-black placeholder:text-black/40 dark:border-white/10 dark:bg-white/5 dark:text-white dark:placeholder:text-white/40"
          />
          <select
            value={sort}
            onChange={(e) => setSort(e.target.value as SortOrder)}
            aria-label="Sort repositories"
            className="rounded-md border border-black/10 bg-white px-3 py-1 text-sm text-black shadow-xs focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-black dark:border-white/10 dark:bg-white/5 dark:text-white dark:focus-visible:ring-white"
          >
            <option value="newest">Newest</option>
            <option value="oldest">Oldest</option>
            <option value="name">Name</option>
          </select>
        </div>

        {/* Repo list */}
        {filteredRepos.length > 0 ? (
          <div className="divide-y divide-black/10 dark:divide-white/10">
            {filteredRepos.map((repo) => (
              <RepoListItem key={repo.id} repo={repo} />
            ))}
          </div>
        ) : (
          <SearchEmptyState />
        )}
      </div>

      {/* Sidebar */}
      <aside className="hidden w-72 shrink-0 border-l border-black/10 pl-8 dark:border-white/10 lg:block">
        <OrgSidebar repos={repos} />
      </aside>
    </div>
  );
}
