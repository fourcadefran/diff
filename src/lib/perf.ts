// Temporary performance instrumentation. Remove once the lag investigation
// is complete. Every log line is prefixed with [diff-perf] so it is easy to
// grep / copy out of the devtools console.
const PREFIX = "[diff-perf]";

type PerfExtra = Record<string, unknown> | undefined;
export type ExpandAllMetricPhase = "propSync" | "contentMount" | "renderCommit";

export interface ExpandAllSession {
  id: number;
  startedAt: number;
  requestedFileCount: number;
}

export interface ExpandAllCardMetric {
  sessionId: number;
  path: string;
  phase: ExpandAllMetricPhase;
  ms: number;
  contentKind: "text" | "binary";
}

function now(): number {
  if (typeof performance !== "undefined") return performance.now();
  return Date.now();
}

export function perfLog(
  layer: string,
  op: string,
  extra?: PerfExtra,
): void {
  if (extra && Object.keys(extra).length > 0) {
    // eslint-disable-next-line no-console
    console.log(`${PREFIX} ${layer}:${op}`, extra);
  } else {
    // eslint-disable-next-line no-console
    console.log(`${PREFIX} ${layer}:${op}`);
  }
}

export function perfLogJson(
  layer: string,
  op: string,
  extra?: PerfExtra,
): void {
  const payload = extra ?? {};
  // eslint-disable-next-line no-console
  console.log(`${PREFIX} ${layer}:${op} ${JSON.stringify(payload)}`);
}

export function perfTimed<T>(
  layer: string,
  op: string,
  fn: () => T,
  extra?: PerfExtra,
): T {
  const start = now();
  try {
    const result = fn();
    perfLog(layer, op, { ms: +(now() - start).toFixed(2), ...extra });
    return result;
  } catch (err) {
    perfLog(layer, `${op}:error`, {
      ms: +(now() - start).toFixed(2),
      error: String(err),
      ...extra,
    });
    throw err;
  }
}

export async function perfTimedAsync<T>(
  layer: string,
  op: string,
  fn: () => Promise<T>,
  extra?: PerfExtra,
): Promise<T> {
  const start = now();
  try {
    const result = await fn();
    perfLog(layer, op, { ms: +(now() - start).toFixed(2), ...extra });
    return result;
  } catch (err) {
    perfLog(layer, `${op}:error`, {
      ms: +(now() - start).toFixed(2),
      error: String(err),
      ...extra,
    });
    throw err;
  }
}

export function perfMark(): () => number {
  const start = now();
  return () => +(now() - start).toFixed(2);
}

export function summarizePerfEntries(
  durations: Map<string, number>,
  topN = 10,
) {
  const entries = Array.from(durations.entries()).sort((a, b) => b[1] - a[1]);
  const total = entries.reduce((sum, [, ms]) => sum + ms, 0);
  return {
    count: entries.length,
    totalMs: +total.toFixed(2),
    avgMs: entries.length === 0 ? 0 : +(total / entries.length).toFixed(2),
    top: entries.slice(0, topN).map(([key, ms]) => ({
      key,
      ms: +ms.toFixed(2),
    })),
  };
}

/**
 * Collect per-item durations and periodically emit an aggregate summary so
 * logs don't get overwhelmed when we have hundreds of items (e.g. one per
 * diff card). Emits on every flush() call; callers decide when to flush.
 */
export function createPerfAggregator(layer: string, op: string) {
  const durations = new Map<string, number>();
  // Optional per-key context (e.g. line count) recorded alongside each ms.
  // Surfaced in the summary's `top` entries so slow files explain themselves.
  const lines = new Map<string, number>();
  return {
    record(key: string, ms: number, lineCount?: number) {
      // Keep worst-case per key so the slow files surface in the summary.
      const prev = durations.get(key);
      if (prev == null || ms > prev) {
        durations.set(key, ms);
        if (lineCount != null) lines.set(key, lineCount);
      }
    },
    size() {
      return durations.size;
    },
    flush(topN = 10) {
      if (durations.size === 0) return;
      const summary = summarizePerfEntries(durations, topN);
      const top = summary.top.map((entry) => {
        const l = lines.get(entry.key);
        return l != null ? { ...entry, lines: l } : entry;
      });
      perfLog(layer, `${op}:summary`, { ...summary, top });
      durations.clear();
      lines.clear();
    },
  };
}
