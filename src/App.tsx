import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  ResizablePanelGroup,
  ResizablePanel,
  ResizableHandle,
} from "@/components/ui/resizable";
import { Toaster } from "@/components/ui/sonner";
import { Sidebar } from "@/components/sidebar/sidebar";
import { DiffPanel } from "@/components/diff-panel/diff-panel";
import { Onboarding } from "@/components/onboarding/onboarding";
import { useRepoStatus } from "@/hooks/use-repo-status";
import { useDiffs } from "@/hooks/use-diffs";
import { useComments } from "@/hooks/use-comments";
import { ask } from "@tauri-apps/plugin-dialog";
import {
  stageFile,
  unstageFile,
  stageAll,
  unstageAll,
  commit,
  submitReview,
  discardFile,
  type CommitOptions,
  type FileEntry,
} from "@/lib/tauri";
import { toast } from "sonner";
import { listen } from "@tauri-apps/api/event";
import type { CommentStatus } from "@/types/comments";
import { perfLog, perfLogJson, type ExpandAllSession } from "@/lib/perf";
import { getWindowKind, getRepoParams } from "@/lib/window";
import { Picker } from "@/components/picker/picker";

interface CommentStatusPayload {
  review_id: string;
  comment_id: string;
  status: CommentStatus;
  summary: string | null;
  dismiss_reason: string | null;
}

const AUTO_EXPAND_FILE_LIMIT = 100;

function App() {
  const kind = getWindowKind();
  if (kind === "picker") {
    return <Picker />;
  }
  return <RepoApp />;
}

function RepoApp() {
  const { workdir, status, error, refresh, open, openRemote, close } = useRepoStatus();
  const { diffs, loading } = useDiffs(status?.staged, status?.unstaged);
  const comments = useComments();
  const [diffStyle, setDiffStyle] = useState<"unified" | "split">("split");
  const [allExpanded, setAllExpanded] = useState(false);
  const [expandAllSession, setExpandAllSession] =
    useState<ExpandAllSession | null>(null);
  const [scrollToPath, setScrollToPath] = useState<string | null>(null);
  const [scrollNonce, setScrollNonce] = useState(0);
  const [submittingReview, setSubmittingReview] = useState(false);
  const [optimisticStage, setOptimisticStage] = useState<Map<string, boolean>>(
    new Map(),
  );
  const expandSessionIdRef = useRef(0);
  const restoreOpenStartedRef = useRef(false);

  // Listen for real-time comment status updates from the Tauri event bridge
  const { updateCommentStatus } = comments;
  useEffect(() => {
    const promise = listen<CommentStatusPayload>(
      "review:comment-updated",
      (event) => {
        updateCommentStatus(
          event.payload.comment_id,
          event.payload.status,
          event.payload.summary,
          event.payload.dismiss_reason,
        );
      },
    );
    return () => {
      promise.then((unlisten) => unlisten());
    };
  }, [updateCommentStatus]);

  // Keep a latest-ref for the repo:changed callback so the Tauri subscription
  // doesn't tear down + re-attach every time `loading` or comment counts
  // change (which would drop fs events landing during re-subscribe).
  const repoChangedRef = useRef<() => void>(() => {});
  useEffect(() => {
    repoChangedRef.current = () => {
      if (!workdir) {
        perfLog("App", "fileWatcher:skip", { reason: "no-workdir" });
        return;
      }
      if (loading) {
        perfLog("App", "fileWatcher:skip", { reason: "diffs-loading" });
        return;
      }
      if (comments.totalCommentCount === 0) {
        perfLog("App", "fileWatcher:tick");
        refresh();
      } else {
        perfLog("App", "fileWatcher:skip", {
          reason: "open-comments",
          totalCommentCount: comments.totalCommentCount,
        });
      }
    };
  });
  useEffect(() => {
    if (!workdir) return;
    let unlisten: (() => void) | null = null;
    let cancelled = false;
    listen("repo:changed", () => repoChangedRef.current()).then((fn) => {
      if (cancelled) fn();
      else unlisten = fn;
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [workdir]);

  useEffect(() => {
    if (!status) return;
    perfLog("App", "status:apply", {
      staged: status.staged.length,
      unstaged: status.unstaged.length,
      total: status.staged.length + status.unstaged.length,
    });
  }, [status]);

  useEffect(() => {
    perfLog("App", "diffs:change", {
      diffCount: diffs.size,
      loading,
    });
  }, [diffs, loading]);

  useEffect(() => {
    perfLog("App", "allExpanded:change", { allExpanded });
  }, [allExpanded]);

  // Apply optimistic stage toggles to the raw staged/unstaged lists so the
  // sidebar sections reshuffle instantly without waiting for the backend.
  const { stagedView, unstagedView } = useMemo(() => {
    if (!status)
      return { stagedView: [] as FileEntry[], unstagedView: [] as FileEntry[] };
    if (optimisticStage.size === 0) {
      return { stagedView: status.staged, unstagedView: status.unstaged };
    }
    const stagedByPath = new Map(status.staged.map((f) => [f.path, f]));
    const unstagedByPath = new Map(status.unstaged.map((f) => [f.path, f]));
    const nextStaged = new Map(stagedByPath);
    const nextUnstaged = new Map(unstagedByPath);
    optimisticStage.forEach((wantStaged, path) => {
      const source = stagedByPath.get(path) ?? unstagedByPath.get(path);
      if (!source) return;
      if (wantStaged) {
        nextStaged.set(path, source);
        nextUnstaged.delete(path);
      } else {
        nextUnstaged.set(path, source);
        nextStaged.delete(path);
      }
    });
    return {
      stagedView: Array.from(nextStaged.values()),
      unstagedView: Array.from(nextUnstaged.values()),
    };
  }, [status, optimisticStage]);

  const stagedPaths = useMemo(
    () => new Set(stagedView.map((f) => f.path)),
    [stagedView],
  );

  const allFiles = useMemo((): FileEntry[] => {
    if (!status) return [];
    const seen = new Set<string>();
    const files: FileEntry[] = [];
    for (const f of status.staged) {
      if (!seen.has(f.path)) {
        seen.add(f.path);
        files.push(f);
      }
    }
    for (const f of status.unstaged) {
      if (!seen.has(f.path)) {
        seen.add(f.path);
        files.push(f);
      }
    }
    return files;
  }, [status]);

  const autoExpandedRepoRef = useRef<string | null>(null);
  useEffect(() => {
    if (!workdir) {
      autoExpandedRepoRef.current = null;
      setExpandAllSession(null);
      return;
    }
    if (!status) return;
    if (autoExpandedRepoRef.current === workdir) return;
    autoExpandedRepoRef.current = workdir;
    const totalFiles = allFiles.length;
    const shouldExpand = totalFiles <= AUTO_EXPAND_FILE_LIMIT;
    perfLog("App", "allExpanded:auto", {
      totalFiles,
      shouldExpand,
      limit: AUTO_EXPAND_FILE_LIMIT,
    });
    setExpandAllSession(null);
    setAllExpanded(shouldExpand);
  }, [allFiles.length, status, workdir]);

  const handleToggleExpandAll = useCallback(() => {
    if (allExpanded) {
      perfLogJson("ExpandAll", "collapseClick", {
        activeSessionId: expandAllSession?.id ?? null,
        totalFiles: allFiles.length,
      });
      setExpandAllSession(null);
      setAllExpanded(false);
      return;
    }
    const nextSession: ExpandAllSession = {
      id: ++expandSessionIdRef.current,
      startedAt: performance.now(),
      requestedFileCount: allFiles.length,
    };
    perfLogJson("ExpandAll", "click", {
      sessionId: nextSession.id,
      totalFiles: allFiles.length,
      loadedDiffs: diffs.size,
      loading,
      diffStyle,
    });
    setExpandAllSession(nextSession);
    setAllExpanded(true);
  }, [
    allExpanded,
    allFiles.length,
    diffStyle,
    diffs.size,
    expandAllSession?.id,
    loading,
  ]);

  const handleSelectFile = useCallback((path: string) => {
    setScrollToPath(path);
    setScrollNonce((n) => n + 1);
  }, []);

  const handleScrollComplete = useCallback(() => {
    setScrollToPath(null);
  }, []);

  const stagedPathsRef = useRef(stagedPaths);
  stagedPathsRef.current = stagedPaths;

  const handleToggleStage = useCallback(
    async (path: string) => {
      const willStage = !stagedPathsRef.current.has(path);
      setOptimisticStage((prev) => {
        const next = new Map(prev);
        next.set(path, willStage);
        return next;
      });
      try {
        if (willStage) {
          await stageFile(path);
        } else {
          await unstageFile(path);
        }
        await refresh();
      } catch (e) {
        toast.error(`Stage failed: ${e}`);
      } finally {
        setOptimisticStage((prev) => {
          if (!prev.has(path)) return prev;
          const next = new Map(prev);
          next.delete(path);
          return next;
        });
      }
    },
    [refresh],
  );

  const handleStageAll = useCallback(async () => {
    const paths = unstagedView.map((f) => f.path);
    if (paths.length === 0) return;
    setOptimisticStage((prev) => {
      const next = new Map(prev);
      for (const path of paths) next.set(path, true);
      return next;
    });

    try {
      await stageAll();
      await refresh();
    } catch (e) {
      toast.error(`Stage all failed: ${e}`);
    } finally {
      setOptimisticStage((prev) => {
        let changed = false;
        const next = new Map(prev);
        for (const path of paths) {
          changed = next.delete(path) || changed;
        }
        return changed ? next : prev;
      });
    }
  }, [refresh, unstagedView]);

  const handleUnstageAll = useCallback(async () => {
    try {
      await unstageAll();
      await refresh();
    } catch (e) {
      toast.error(`Unstage all failed: ${e}`);
    }
  }, [refresh]);

  const handleCommit = useCallback(
    async (message: string, options?: CommitOptions) => {
      try {
        const oid = await commit(message, options);
        toast.success(
          `${options?.amend ? "Amended" : "Committed"}: ${oid.slice(0, 7)}`,
        );
        await refresh();
      } catch (e) {
        toast.error(`Commit failed: ${e}`);
      }
    },
    [refresh],
  );

  const handleDiscardFile = useCallback(
    async (path: string) => {
      const ok = await ask(
        `Discard changes to ${path}? This cannot be undone.`,
        { title: "Discard changes", kind: "warning" },
      );
      if (!ok) return;
      try {
        await discardFile(path);
        await refresh();
        toast.success(`Discarded ${path}`);
      } catch (e) {
        toast.error(`Discard failed: ${e}`);
      }
    },
    [refresh],
  );

  const { collectAllComments, markSubmitted } = comments;
  const submittingRef = useRef(false);

  const handleSubmitReview = useCallback(async () => {
    if (submittingRef.current) return;
    const reviewComments = collectAllComments();
    if (reviewComments.length === 0) return;

    submittingRef.current = true;
    setSubmittingReview(true);
    try {
      const result = await submitReview(reviewComments);
      toast.success(`Submitted ${result.submitted_count} comment(s)`);
      // Map server IDs back to local annotations by key
      const idMap = new Map(result.comment_ids.map((m) => [m.key, m.id]));
      markSubmitted(idMap);
    } catch (e) {
      toast.error(`Review submit failed: ${e}`);
    } finally {
      submittingRef.current = false;
      setSubmittingReview(false);
    }
  }, [collectAllComments, markSubmitted]);

  // Open the repo this window was spawned for (host/path in URL query).
  const openRef = useRef(open);
  openRef.current = open;
  const openRemoteRef = useRef(openRemote);
  openRemoteRef.current = openRemote;
  useEffect(() => {
    const params = getRepoParams();
    if (!params || restoreOpenStartedRef.current) return;
    restoreOpenStartedRef.current = true;
    perfLog("App", "open:restore", {
      source: "url",
      host: params.host,
      path: params.path,
    });
    if (params.host === "local") {
      openRef.current(params.path).catch((e) =>
        toast.error(`Failed to open: ${e}`),
      );
    } else {
      openRemoteRef.current(params.host, params.path).catch((e) => {
        const msg = String(e);
        if (msg.includes("Could not connect to diff-agent")) {
          toast.error(msg, { duration: 30000 });
        } else {
          toast.error(`Connect failed: ${msg}`);
        }
      });
    }
  }, []);

  if (!workdir) {
    return (
      <>
        <Onboarding onOpened={open} />
        <Toaster />
      </>
    );
  }

  if (error) {
    return (
      <main className="flex h-dvh items-center justify-center p-4">
        <p className="text-destructive text-sm">{error}</p>
      </main>
    );
  }

  if (!status) {
    return (
      <main className="flex h-dvh items-center justify-center">
        <p className="text-muted-foreground text-sm">Loading...</p>
      </main>
    );
  }

  return (
    <>
      <ResizablePanelGroup
        orientation="horizontal"
        className="h-full isolate border-t border-border bg-background"
      >
        <ResizablePanel defaultSize="25%" minSize={300} maxSize={400}>
          <Sidebar
            workdir={workdir}
            staged={stagedView}
            unstaged={unstagedView}
            stagedPaths={stagedPaths}
            selectedFile={scrollToPath}
            onSelectFile={handleSelectFile}
            onToggleStage={handleToggleStage}
            onStageAll={handleStageAll}
            onUnstageAll={handleUnstageAll}
            onCommit={handleCommit}
            onCloseRepo={close}
            onDiscardFile={handleDiscardFile}
          />
        </ResizablePanel>
        <ResizableHandle />
        <ResizablePanel defaultSize="78%">
          <DiffPanel
            files={allFiles}
            diffs={diffs}
            loading={loading}
            diffStyle={diffStyle}
            onDiffStyleChange={setDiffStyle}
            allExpanded={allExpanded}
            onToggleExpandAll={handleToggleExpandAll}
            expandAllSession={expandAllSession}
            scrollToPath={scrollToPath}
            scrollNonce={scrollNonce}
            onScrollComplete={handleScrollComplete}
            annotationsByFile={comments.annotationsByFile}
            hasOpenForm={comments.hasOpenForm}
            totalCommentCount={comments.totalCommentCount}
            pendingCount={comments.pendingCount}
            acknowledgedCount={comments.acknowledgedCount}
            resolvedCount={comments.resolvedCount}
            onAddAnnotation={comments.addFormAnnotation}
            onCancelAnnotation={comments.cancelAnnotation}
            onSubmitAnnotation={comments.submitAnnotation}
            onDeleteAnnotation={comments.deleteAnnotation}
            onSubmitReview={handleSubmitReview}
            onClearResolved={comments.clearResolved}
            submittingReview={submittingReview}
          />
        </ResizablePanel>
      </ResizablePanelGroup>
      <Toaster />
    </>
  );
}

export default App;
