export type WindowKind = "picker" | "repo";

/// Read window params from either the query string (used by the picker, set
/// in tauri.conf.json) or the hash fragment (used by programmatically-created
/// repo windows — hash survives PathBuf-based URL building in Tauri).
function readParams(): URLSearchParams {
  if (window.location.hash.length > 1) {
    return new URLSearchParams(window.location.hash.slice(1));
  }
  return new URLSearchParams(window.location.search);
}

export function getWindowKind(): WindowKind {
  return readParams().get("kind") === "picker" ? "picker" : "repo";
}

export function getRepoParams(): { host: string; path: string } | null {
  const params = readParams();
  const host = params.get("host");
  const path = params.get("path");
  if (!host || !path) return null;
  return { host, path };
}
