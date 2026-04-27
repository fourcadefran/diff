export type WindowKind = "picker" | "repo";

export function getWindowKind(): WindowKind {
  const params = new URLSearchParams(window.location.search);
  return params.get("kind") === "picker" ? "picker" : "repo";
}

export function getRepoParams(): { host: string; path: string } | null {
  const params = new URLSearchParams(window.location.search);
  const host = params.get("host");
  const path = params.get("path");
  if (!host || !path) return null;
  return { host, path };
}
