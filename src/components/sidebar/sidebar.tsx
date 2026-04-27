import {
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
} from "react";
import {
  FileTree,
  useFileTree,
  useFileTreeSelection,
} from "@pierre/trees/react";
import type { GitStatus, GitStatusEntry } from "@pierre/trees";
import { Button } from "@/components/ui/button";
import { CommitBar } from "./commit-bar";
import { SidebarContextMenu } from "./sidebar-context-menu";
import type { ChangeKind, CommitOptions, FileEntry } from "@/lib/tauri";
import { toast } from "sonner";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { join } from "@tauri-apps/api/path";
import { IconArrowLeft } from "@tabler/icons-react";
import { perfTimed } from "@/lib/perf";

interface SidebarProps {
  workdir: string | null;
  staged: FileEntry[];
  unstaged: FileEntry[];
  stagedPaths: Set<string>;
  selectedFile: string | null;
  onSelectFile: (path: string) => void;
  onToggleStage: (path: string) => void;
  onStageAll: () => void;
  onUnstageAll: () => void;
  onCommit: (message: string, options?: CommitOptions) => void;
  onCloseRepo: () => void;
  onDiscardFile: (path: string) => void;
}

const treeStyle: CSSProperties = {
  colorScheme: "dark",
  "--trees-bg-override": "transparent",
  "--trees-fg-override": "var(--foreground)",
  "--trees-fg-muted-override": "var(--muted-foreground)",
  "--trees-bg-muted-override": "var(--muted)",
  "--trees-selected-bg-override": "var(--accent)",
  "--trees-selected-fg-override": "var(--accent-foreground)",
  "--trees-border-color-override": "var(--border)",
  "--trees-padding-inline-override": "6px",
  "--trees-item-margin-x-override": "0px",
  height: "100%",
} as CSSProperties;

function mapKind(kind: ChangeKind): GitStatus {
  switch (kind) {
    case "added":
      return "added";
    case "deleted":
      return "deleted";
    case "renamed":
      return "renamed";
    case "modified":
    case "typechange":
    default:
      return "modified";
  }
}

export function Sidebar({
  workdir,
  staged,
  unstaged,
  stagedPaths,
  onSelectFile,
  onToggleStage,
  onStageAll,
  onUnstageAll,
  onCommit,
  onCloseRepo,
  onDiscardFile,
}: SidebarProps) {
  const hasChanges = staged.length > 0 || unstaged.length > 0;
  const totalCount = staged.length + unstaged.length;

  return (
    <div className="flex h-full w-full min-w-0 flex-col overflow-hidden border-r border-border bg-sidebar">
      <div className="flex h-10 items-center gap-1 border-b border-border px-1.5">
        <Button
          variant="ghost"
          size="icon-xs"
          onClick={onCloseRepo}
          aria-label="Back to onboarding"
          title="Open a different repository"
        >
          <IconArrowLeft />
        </Button>
        <p className="min-w-0 flex-1 truncate text-sm font-medium text-sidebar-foreground">
          {workdir?.replace(/\/+$/, "").split("/").pop() ?? "No repository"}
        </p>
        <p className="shrink-0 pr-1 text-xs tabular-nums text-muted-foreground">
          {totalCount} change{totalCount === 1 ? "" : "s"}
        </p>
      </div>

      <div className="flex min-h-0 flex-1 flex-col overflow-y-auto overflow-x-hidden scrollbar-hide">
        {!hasChanges && (
          <p className="px-3 py-4 text-xs text-muted-foreground">No changes</p>
        )}

        {staged.length > 0 && (
          <Section
            label="Staged"
            files={staged}
            treeId="diff-staged-tree"
            stagedPaths={stagedPaths}
            actionLabel="Unstage All"
            onAction={onUnstageAll}
            onSelectFile={onSelectFile}
            onToggleStage={onToggleStage}
            onDiscardFile={onDiscardFile}
            workdir={workdir}
          />
        )}
        {unstaged.length > 0 && (
          <Section
            label="Unstaged"
            files={unstaged}
            treeId="diff-unstaged-tree"
            stagedPaths={stagedPaths}
            actionLabel="Stage All"
            onAction={onStageAll}
            onSelectFile={onSelectFile}
            onToggleStage={onToggleStage}
            onDiscardFile={onDiscardFile}
            workdir={workdir}
          />
        )}
      </div>

      <CommitBar stagedCount={staged.length} onCommit={onCommit} />
    </div>
  );
}

interface SectionProps {
  label: string;
  files: FileEntry[];
  treeId: string;
  stagedPaths: Set<string>;
  actionLabel: string;
  onAction: () => void;
  onSelectFile: (path: string) => void;
  onToggleStage: (path: string) => void;
  onDiscardFile: (path: string) => void;
  workdir: string | null;
}

function Section({
  label,
  files,
  treeId,
  stagedPaths,
  actionLabel,
  onAction,
  onSelectFile,
  onToggleStage,
  onDiscardFile,
  workdir,
}: SectionProps) {
  const paths = useMemo(() => files.map((f) => f.path), [files]);
  const gitStatus = useMemo<GitStatusEntry[]>(
    () => files.map((f) => ({ path: f.path, status: mapKind(f.kind) })),
    [files],
  );

  const { model } = useFileTree({
    id: treeId,
    paths,
    flattenEmptyDirectories: true,
    initialExpansion: "open",
    gitStatus,
    density: "compact",
    composition: {
      contextMenu: {
        enabled: true,
        triggerMode: "right-click",
      },
    },
  });

  useEffect(() => {
    perfTimed("Sidebar", "model.resetPaths", () => model.resetPaths(paths), {
      treeId,
      count: paths.length,
    });
  }, [paths, model, treeId]);

  useEffect(() => {
    perfTimed(
      "Sidebar",
      "model.setGitStatus",
      () => model.setGitStatus(gitStatus),
      { treeId, count: gitStatus.length },
    );
  }, [gitStatus, model, treeId]);

  const selectedPaths = useFileTreeSelection(model);
  const lastEmittedRef = useRef<string | null>(null);
  useEffect(() => {
    if (selectedPaths.length === 0) {
      lastEmittedRef.current = null;
      return;
    }
    const path = selectedPaths[selectedPaths.length - 1];
    if (path === lastEmittedRef.current) return;
    const item = model.getItem(path);
    if (item == null || item.isDirectory()) return;
    lastEmittedRef.current = path;
    onSelectFile(path);
  }, [selectedPaths, model, onSelectFile]);

  // Size each section to its rendered tree height so the outer sidebar owns the
  // single scroll (VSCode source-control style) instead of giving each tree its
  // own viewport.
  const treeWrapperRef = useRef<HTMLDivElement>(null);
  const itemHeight = model.getItemHeight();
  const [treeHeight, setTreeHeight] = useState(() => files.length * itemHeight);

  useLayoutEffect(() => {
    const wrapper = treeWrapperRef.current;
    if (wrapper == null) return;
    let rafId = 0;
    let mutationObserver: MutationObserver | null = null;
    let cancelled = false;
    const attach = () => {
      if (cancelled) return;
      const host = wrapper.querySelector(
        "file-tree-container",
      ) as HTMLElement | null;
      const list = host?.shadowRoot?.querySelector<HTMLElement>(
        '[data-file-tree-virtualized-list="true"]',
      );
      if (list == null) {
        rafId = requestAnimationFrame(attach);
        return;
      }
      // The tree sets `style.height = ${totalScrollableHeight}px` on the list
      // whenever rows are added, removed, collapsed, or expanded. Observing the
      // rendered box (ResizeObserver) misses collapses because the list's
      // `min-height: 100%` pins it to the (stale) wrapper height, so we watch
      // the inline style attribute instead.
      const update = () => {
        const h = parseFloat(list.style.height);
        if (Number.isFinite(h) && h > 0) setTreeHeight(Math.ceil(h));
      };
      update();
      mutationObserver = new MutationObserver(update);
      mutationObserver.observe(list, {
        attributes: true,
        attributeFilter: ["style"],
      });
    };
    attach();
    return () => {
      cancelled = true;
      if (rafId) cancelAnimationFrame(rafId);
      mutationObserver?.disconnect();
    };
  }, [model]);

  // Catch every file-row click (including re-clicks of the already-selected
  // row). `useFileTreeSelection` is memoized by array equality, so clicking
  // a selected row does not fire `onSelectionChange` — which would leave
  // the diff panel stale when the user wants to re-open a collapsed card.
  useEffect(() => {
    const wrapper = treeWrapperRef.current;
    if (wrapper == null) return;
    const handleClick = (event: MouseEvent) => {
      for (const el of event.composedPath()) {
        if (!(el instanceof HTMLElement)) continue;
        if (el.dataset.itemType === "file") {
          const itemPath = el.dataset.itemPath;
          if (itemPath) onSelectFile(itemPath);
          return;
        }
      }
    };
    wrapper.addEventListener("click", handleClick);
    return () => wrapper.removeEventListener("click", handleClick);
  }, [onSelectFile]);

  return (
    <div className="flex shrink-0 flex-col">
      <div className="flex items-center justify-between px-2 py-1">
        <span className="text-xs font-medium text-muted-foreground">
          {label} <span className="text-[10px]">({files.length})</span>
        </span>
        <Button
          variant="ghost"
          size="sm"
          className="h-5 px-1.5 text-[10px]"
          onClick={onAction}
        >
          {actionLabel}
        </Button>
      </div>
      <div
        ref={treeWrapperRef}
        className="shrink-0"
        style={{ height: treeHeight }}
      >
        <FileTree
          model={model}
          style={treeStyle}
          renderContextMenu={(item, context) => (
            <SidebarContextMenu
              item={item}
              context={context}
              isStaged={stagedPaths.has(item.path)}
              onStage={(p) => {
                context.close();
                onToggleStage(p);
              }}
              onUnstage={(p) => {
                context.close();
                onToggleStage(p);
              }}
              onDiscard={(p) => {
                context.close();
                onDiscardFile(p);
              }}
              onCopyPath={(p) => {
                context.close();
                navigator.clipboard.writeText(p).catch(() => {});
              }}
              onRevealInFinder={(p) => {
                context.close();
                if (!workdir) return;
                join(workdir, p)
                  .then(revealItemInDir)
                  .catch((e) => toast.error(`Reveal failed: ${e}`));
              }}
            />
          )}
        />
      </div>
    </div>
  );
}
