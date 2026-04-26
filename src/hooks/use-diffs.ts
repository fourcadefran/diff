import { useEffect, useMemo, useRef, useState } from "react";
import {
  getFileContentsBatch,
  type FileContentsRequest,
  type FileContentsResponse,
  type FileEntry,
} from "@/lib/tauri";
import { perfLog } from "@/lib/perf";

type TextFileDiffContents = {
  kind: "text";
  oldFile: { name: string; contents: string };
  newFile: { name: string; contents: string };
};

type BinaryFileDiffContents = {
  kind: "binary";
  oldBinary: boolean;
  newBinary: boolean;
};

export type FileDiffContents = TextFileDiffContents | BinaryFileDiffContents;

interface UseDiffsReturn {
  diffs: Map<string, FileDiffContents>;
  loading: boolean;
}

export function useDiffs(
  staged: FileEntry[] | undefined,
  unstaged: FileEntry[] | undefined,
): UseDiffsReturn {
  const [diffs, setDiffs] = useState<Map<string, FileDiffContents>>(new Map());
  const [loading, setLoading] = useState(false);
  const diffsRef = useRef(diffs);
  diffsRef.current = diffs;
  const diffSidesRef = useRef(new Map<string, boolean>());

  const requests = useMemo<FileContentsRequest[]>(() => {
    if (!staged || !unstaged) return [];
    const seen = new Map<string, boolean>();
    for (const f of staged) seen.set(f.path, true);
    for (const f of unstaged) {
      if (!seen.has(f.path)) seen.set(f.path, false);
    }
    const arr: FileContentsRequest[] = Array.from(seen, ([path, isStaged]) => ({
      path,
      staged: isStaged,
    }));
    arr.sort((a, b) => (a.path < b.path ? -1 : a.path > b.path ? 1 : 0));
    return arr;
  }, [staged, unstaged]);

  const requestsKey = useMemo(() => {
    if (!staged || !unstaged) return null;
    return requests
      .map((r) => `${r.staged ? "1" : "0"}${r.path}`)
      .join("\0");
  }, [requests, staged, unstaged]);

  // Latest requests read inside the effect; deps stay on the string key so
  // the effect only re-runs when the set actually changes.
  const requestsRef = useRef(requests);
  requestsRef.current = requests;

  useEffect(() => {
    if (requestsKey == null) return;

    if (requestsKey === "") {
      if (diffsRef.current.size > 0) setDiffs(new Map());
      diffSidesRef.current.clear();
      setLoading(false);
      return;
    }

    const requests = requestsRef.current;
    const pathSet = new Set<string>();
    for (const r of requests) pathSet.add(r.path);
    const current = diffsRef.current;
    const currentSides = diffSidesRef.current;
    const missing = requests.filter(
      (request) =>
        !current.has(request.path) ||
        currentSides.get(request.path) !== request.staged,
    );

    // Prune entries for paths no longer present.
    let needsPrune = current.size !== pathSet.size;
    if (!needsPrune) {
      for (const p of current.keys()) {
        if (!pathSet.has(p)) {
          needsPrune = true;
          break;
        }
      }
    }
    if (needsPrune) {
      const pruned = new Map<string, FileDiffContents>();
      for (const [p, v] of current) {
        if (pathSet.has(p)) pruned.set(p, v);
      }
      for (const p of currentSides.keys()) {
        if (!pathSet.has(p)) currentSides.delete(p);
      }
      setDiffs(pruned);
    }

    perfLog("useDiffs", "effect:run", {
      totalPaths: pathSet.size,
      cached: pathSet.size - missing.length,
      missing: missing.length,
      pruned: needsPrune,
    });

    if (missing.length === 0) {
      setLoading(false);
      return;
    }

    let cancelled = false;

    setLoading(true);
    const batchStart = performance.now();
    let okCount = 0;
    let errCount = 0;
    const requestedSides = new Map(
      missing.map((request) => [request.path, request.staged]),
    );

    void getFileContentsBatch(missing)
      .then((results) => {
        if (cancelled) return;
        // Precompute the side-effects outside the setState updater so React
        // is free to re-invoke the updater (StrictMode / concurrent) without
        // double-mutating `currentSides` or double-counting telemetry.
        const parsed: Array<{ path: string; value: FileDiffContents }> = [];
        const deleted: string[] = [];
        for (const result of results) {
          if (result.response) {
            okCount += 1;
            parsed.push({
              path: result.path,
              value: toFileDiffContents(result.response),
            });
            currentSides.set(
              result.path,
              requestedSides.get(result.path) ?? false,
            );
          } else {
            errCount += 1;
            console.warn(
              "[diff] failed to fetch diff:",
              result.error ?? result.path,
            );
            deleted.push(result.path);
            currentSides.delete(result.path);
          }
        }
        setDiffs((prev) => {
          let changed = false;
          const next = new Map(prev);
          for (const { path, value } of parsed) {
            if (next.get(path) !== value) changed = true;
            next.set(path, value);
          }
          for (const path of deleted) {
            if (next.delete(path)) changed = true;
          }
          return changed ? next : prev;
        });
      })
      .catch((err) => {
        if (cancelled) return;
        errCount = missing.length;
        console.warn("[diff] failed to fetch diff batch:", err);
        for (const request of missing) currentSides.delete(request.path);
        setDiffs((prev) => {
          let changed = false;
          const next = new Map(prev);
          for (const request of missing) {
            if (next.delete(request.path)) changed = true;
          }
          return changed ? next : prev;
        });
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
        const totalMs = +(performance.now() - batchStart).toFixed(2);
        perfLog("useDiffs", "getFileContents:batchDone", {
          requested: missing.length,
          ok: okCount,
          errors: errCount,
          totalMs,
          cancelled,
        });
      });

    return () => {
      cancelled = true;
    };
  }, [requestsKey]);

  return { diffs, loading };
}

function toFileDiffContents(resp: FileContentsResponse): FileDiffContents {
  if (resp.old_binary || resp.new_binary) {
    return {
      kind: "binary",
      oldBinary: resp.old_binary,
      newBinary: resp.new_binary,
    };
  }

  return {
    kind: "text",
    oldFile: { name: resp.name, contents: resp.old_content ?? "" },
    newFile: { name: resp.name, contents: resp.new_content ?? "" },
  };
}
