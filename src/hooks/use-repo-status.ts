import { useCallback, useRef, useState } from "react";
import {
  openRepo,
  getRepoStatus,
  type FileEntry,
  type RepoStatus,
} from "@/lib/tauri";
import { perfLog, perfMark, perfTimedAsync } from "@/lib/perf";

interface MergedRepoStatus {
  staged: FileEntry[];
  unstaged: FileEntry[];
}

interface UseRepoStatusReturn {
  workdir: string | null;
  status: MergedRepoStatus | null;
  error: string | null;
  refresh: () => Promise<void>;
  open: (path: string) => Promise<void>;
  close: () => void;
}

const LAST_OPENED_REPO_KEY = "diff:last-opened-repo";

export function readLastOpenedRepo(): string | null {
  if (typeof window === "undefined") return null;
  try {
    const path = window.localStorage.getItem(LAST_OPENED_REPO_KEY);
    return path && path.trim() ? path : null;
  } catch {
    return null;
  }
}

export function clearLastOpenedRepo(): void {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.removeItem(LAST_OPENED_REPO_KEY);
  } catch {
    // ignore
  }
}

function writeLastOpenedRepo(path: string): void {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(LAST_OPENED_REPO_KEY, path);
  } catch {
    // ignore
  }
}

function mergeStatus(raw: RepoStatus): MergedRepoStatus {
  const unstaged: FileEntry[] = [
    ...raw.unstaged,
    ...raw.untracked.map(
      (path): FileEntry => ({
        path,
        kind: "added",
        additions: 0,
        deletions: 0,
      }),
    ),
  ];
  return { staged: raw.staged, unstaged };
}

function statusFingerprint(s: MergedRepoStatus): string {
  const entries = (files: FileEntry[]) =>
    files
      .map((f) => `${f.path}:${f.kind}:${f.additions}:${f.deletions}`)
      .join(",");
  return `${entries(s.staged)}|${entries(s.unstaged)}`;
}

export function useRepoStatus(): UseRepoStatusReturn {
  const [workdir, setWorkdir] = useState<string | null>(null);
  const [status, setStatus] = useState<MergedRepoStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const fingerprintRef = useRef("");
  const refreshPromiseRef = useRef<Promise<void> | null>(null);
  const pendingRefreshRef = useRef(false);

  const applyStatus = useCallback((merged: MergedRepoStatus) => {
    const fp = statusFingerprint(merged);
    const totalFiles = merged.staged.length + merged.unstaged.length;
    if (fp === fingerprintRef.current) {
      perfLog("useRepoStatus", "applyStatus:skip", {
        reason: "fingerprint-match",
        totalFiles,
      });
      return;
    }
    fingerprintRef.current = fp;
    perfLog("useRepoStatus", "applyStatus:set", {
      staged: merged.staged.length,
      unstaged: merged.unstaged.length,
      totalFiles,
    });
    setStatus(merged);
  }, []);

  const refresh = useCallback(async () => {
    if (refreshPromiseRef.current) {
      pendingRefreshRef.current = true;
      perfLog("useRepoStatus", "refresh:coalesce");
      return refreshPromiseRef.current;
    }

    const promise = (async () => {
      try {
        do {
          pendingRefreshRef.current = false;
          try {
            setError(null);
            const raw = await perfTimedAsync(
              "useRepoStatus",
              "refresh:getRepoStatus",
              () => getRepoStatus(),
            );
            perfLog("useRepoStatus", "refresh:raw", {
              staged: raw.staged.length,
              unstaged: raw.unstaged.length,
              untracked: raw.untracked.length,
            });
            applyStatus(mergeStatus(raw));
          } catch (e) {
            setError(String(e));
          }
        } while (pendingRefreshRef.current);
      } finally {
        refreshPromiseRef.current = null;
      }
    })();
    refreshPromiseRef.current = promise;
    return promise;
  }, [applyStatus]);

  const open = useCallback(
    async (path: string) => {
      const markOpen = perfMark();
      perfLog("useRepoStatus", "open:start", { path });
      setError(null);
      try {
        const dir = await perfTimedAsync(
          "useRepoStatus",
          "open:openRepo",
          () => openRepo(path),
          { path },
        );
        const raw = await perfTimedAsync(
          "useRepoStatus",
          "open:getRepoStatus",
          () => getRepoStatus(),
        );
        perfLog("useRepoStatus", "open:raw", {
          staged: raw.staged.length,
          unstaged: raw.unstaged.length,
          untracked: raw.untracked.length,
        });
        fingerprintRef.current = "";
        applyStatus(mergeStatus(raw));
        setWorkdir(dir);
        writeLastOpenedRepo(dir);
        perfLog("useRepoStatus", "open:done", {
          path,
          workdir: dir,
          staged: raw.staged.length,
          unstaged: raw.unstaged.length,
          untracked: raw.untracked.length,
          totalFiles: raw.staged.length + raw.unstaged.length + raw.untracked.length,
          ms: markOpen(),
        });
      } catch (e) {
        perfLog("useRepoStatus", "open:error", {
          path,
          ms: markOpen(),
          error: String(e),
        });
        throw e;
      }
    },
    [applyStatus],
  );

  const close = useCallback(() => {
    setWorkdir(null);
    setStatus(null);
    setError(null);
    clearLastOpenedRepo();
    fingerprintRef.current = "";
  }, []);

  return { workdir, status, error, refresh, open, close };
}
