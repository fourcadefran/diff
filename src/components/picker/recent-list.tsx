import { IconFolderOpen, IconGitBranch } from "@tabler/icons-react";
import { invoke } from "@tauri-apps/api/core";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { useEffect, useState } from "react";
import { toast } from "sonner";

interface RecentRepo {
  host: string;
  path: string;
  last_opened_at: number;
}

interface Props {
  host: string;
}

export function RecentList({ host }: Props) {
  const [recents, setRecents] = useState<RecentRepo[]>([]);
  const [remotePathInput, setRemotePathInput] = useState("");

  useEffect(() => {
    invoke<RecentRepo[]>("list_recent_repos", { host })
      .then(setRecents)
      .catch((e) => console.error("[diff-picker] list_recent_repos:", e));
  }, [host]);

  const openRepo = async (path: string) => {
    try {
      await invoke("open_picker_repo", { host, path });
      const fresh = await invoke<RecentRepo[]>("list_recent_repos", { host });
      setRecents(fresh);
    } catch (e) {
      toast.error(`Open failed: ${e}`);
    }
  };

  const onPickLocal = async () => {
    if (host !== "local") {
      toast.error("Local picker only available for Local host");
      return;
    }
    const selected = await openDialog({ directory: true, multiple: false });
    if (typeof selected === "string") {
      openRepo(selected);
    }
  };

  const onOpenRemote = async () => {
    const path = remotePathInput.trim();
    if (!path) return;
    openRepo(path);
    setRemotePathInput("");
  };

  return (
    <div className="flex h-full flex-col gap-3 p-3">
      <div className="flex gap-2">
        {host === "local" ? (
          <button
            type="button"
            onClick={onPickLocal}
            className="flex items-center gap-2 rounded-md bg-primary px-3 py-1.5 text-sm text-primary-foreground hover:opacity-90"
          >
            <IconFolderOpen className="size-4" />
            Open repo…
          </button>
        ) : (
          <div className="flex flex-1 gap-2">
            <input
              type="text"
              placeholder="/path/on/remote/host"
              value={remotePathInput}
              onChange={(e) => setRemotePathInput(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && onOpenRemote()}
              className="flex-1 rounded-md border border-border bg-card px-2 py-1.5 text-sm"
            />
            <button
              type="button"
              onClick={onOpenRemote}
              className="rounded-md bg-primary px-3 py-1.5 text-sm text-primary-foreground hover:opacity-90"
            >
              Open
            </button>
          </div>
        )}
      </div>

      <div className="flex flex-col gap-1 overflow-y-auto">
        <p className="px-2 text-xs uppercase tracking-wide text-muted-foreground">
          Recent
        </p>
        {recents.length === 0 ? (
          <p className="px-2 py-1 text-sm text-muted-foreground">
            No recent repositories.
          </p>
        ) : (
          recents.map((r) => (
            <button
              key={`${r.host}:${r.path}`}
              type="button"
              onClick={() => openRepo(r.path)}
              className="flex items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent"
            >
              <IconGitBranch className="size-4 shrink-0 text-muted-foreground" />
              <span className="truncate">{r.path}</span>
            </button>
          ))
        )}
      </div>
    </div>
  );
}
