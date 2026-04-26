import { useMemo, useState } from "react";
import {
  IconCloudDownload,
  IconFolderOpen,
  IconFolderPlus,
  IconGitBranch,
} from "@tabler/icons-react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { toast } from "sonner";
import { cn } from "@/lib/utils";
import { perfLog, perfTimedAsync } from "@/lib/perf";
import { useRecentRepos } from "@/hooks/use-recent-repos";
import { useRecentBranches } from "@/hooks/use-recent-branches";
import { CloneDialog } from "@/components/onboarding/clone-dialog";
import { CreateDialog } from "@/components/onboarding/create-dialog";

interface OnboardingProps {
  onOpened: (workdir: string) => void | Promise<void>;
}

function basename(path: string): string {
  const trimmed = path.replace(/[\\/]+$/, "");
  const idx = Math.max(trimmed.lastIndexOf("/"), trimmed.lastIndexOf("\\"));
  return idx === -1 ? trimmed : trimmed.slice(idx + 1);
}

function parentPath(path: string): string {
  const trimmed = path.replace(/[\\/]+$/, "");
  const idx = Math.max(trimmed.lastIndexOf("/"), trimmed.lastIndexOf("\\"));
  return idx <= 0 ? "" : trimmed.slice(0, idx);
}

export function Onboarding({ onOpened }: OnboardingProps) {
  const { recent, addRecent, removeRecent } = useRecentRepos();
  const recentPaths = useMemo(() => recent.map((r) => r.path), [recent]);
  const branchByPath = useRecentBranches(recentPaths);
  const [cloneOpen, setCloneOpen] = useState(false);
  const [createOpen, setCreateOpen] = useState(false);

  const openPath = async (path: string) => {
    try {
      await perfTimedAsync(
        "Onboarding",
        "openPath:onOpened",
        () => Promise.resolve(onOpened(path)),
        { path },
      );
      addRecent(path);
    } catch (e) {
      toast.error(`Failed to open: ${e}`);
    }
  };

  const handleOpenLocal = async () => {
    try {
      const selected = await openDialog({ directory: true, multiple: false });
      if (typeof selected !== "string") {
        perfLog("Onboarding", "openLocal:cancel");
        return;
      }
      perfLog("Onboarding", "openLocal:selected", { path: selected });
      await openPath(selected);
    } catch (e) {
      toast.error(`Open failed: ${e}`);
    }
  };

  return (
    <main className="relative h-dvh overflow-y-auto bg-background text-foreground">
      <div className="mx-auto flex max-w-[720px] flex-col gap-6 px-6 pt-24 pb-16">
        <header className="flex flex-col gap-1">
          <h1 className="font-heading text-3xl font-semibold tracking-tight">
            diff
          </h1>
          <p className="text-sm text-muted-foreground">
            Open a repository to start reviewing.
          </p>
        </header>

        <div className="flex gap-3">
          <ActionCard
            primary
            icon={<IconFolderOpen className="size-5" />}
            label="Open Local Repository"
            description="Pick a folder on disk."
            onClick={handleOpenLocal}
          />
          <ActionCard
            icon={<IconCloudDownload className="size-5" />}
            label="Clone from Remote"
            description="Clone a git URL."
            onClick={() => setCloneOpen(true)}
          />
          <ActionCard
            icon={<IconFolderPlus className="size-5" />}
            label="Create New Repository"
            description="Initialize a new repo."
            onClick={() => setCreateOpen(true)}
          />
        </div>

        <section className="flex flex-col gap-2">
          <h2 className="text-xs uppercase tracking-wide text-muted-foreground">
            Recent
          </h2>
          {recent.length === 0 ? (
            <p className="text-sm text-muted-foreground">
              No recent repositories yet.
            </p>
          ) : (
            <ul className="flex flex-col">
              {recent.map((r) => {
                const branch = branchByPath[r.path];
                return (
                  <li key={r.path}>
                    <button
                      type="button"
                      onClick={() => openPath(r.path)}
                      onAuxClick={(e) => {
                        if (e.button === 1) removeRecent(r.path);
                      }}
                      className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left transition-colors hover:bg-accent"
                    >
                      <IconGitBranch className="size-4 shrink-0 text-muted-foreground" />
                      <span className="text-sm font-medium">
                        {basename(r.path)}
                      </span>
                      <span className="truncate text-xs text-muted-foreground">
                        {parentPath(r.path)}
                      </span>
                      <span className="ml-auto shrink-0 font-mono text-xs text-muted-foreground">
                        {branch === undefined ? "" : branch ?? "—"}
                      </span>
                    </button>
                  </li>
                );
              })}
            </ul>
          )}
        </section>
      </div>

      <CloneDialog
        open={cloneOpen}
        onOpenChange={setCloneOpen}
        onCloned={openPath}
      />
      <CreateDialog
        open={createOpen}
        onOpenChange={setCreateOpen}
        onCreated={openPath}
      />
    </main>
  );
}

interface ActionCardProps {
  primary?: boolean;
  icon: React.ReactNode;
  label: string;
  description: string;
  onClick: () => void;
}

function ActionCard({
  primary,
  icon,
  label,
  description,
  onClick,
}: ActionCardProps) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        "flex flex-1 flex-col gap-3 rounded-xl border p-4 text-left transition-colors",
        primary
          ? "border-transparent bg-primary text-primary-foreground hover:bg-primary/90"
          : "border-border bg-card hover:bg-accent",
      )}
    >
      <span
        className={cn(
          primary ? "text-primary-foreground" : "text-foreground",
        )}
      >
        {icon}
      </span>
      <span className="flex flex-col gap-1">
        <span className="text-sm font-medium">{label}</span>
        <span
          className={cn(
            "text-xs",
            primary
              ? "text-primary-foreground/70"
              : "text-muted-foreground",
          )}
        >
          {description}
        </span>
      </span>
    </button>
  );
}
