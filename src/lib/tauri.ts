import { invoke } from "@tauri-apps/api/core";
import type { ReviewComment } from "@/types/comments";
export type ChangeKind =
  | "added"
  | "modified"
  | "deleted"
  | "renamed"
  | "typechange";

export interface FileEntry {
  path: string;
  kind: ChangeKind;
  additions: number;
  deletions: number;
}

export interface RepoStatus {
  staged: FileEntry[];
  unstaged: FileEntry[];
  untracked: string[];
}

export function openRepo(path: string): Promise<string> {
  return invoke<string>("open_repo", { path });
}

export function getRepoStatus(): Promise<RepoStatus> {
  return invoke<RepoStatus>("get_repo_status");
}

export interface FileContentsResponse {
  name: string;
  old_content: string | null;
  old_binary: boolean;
  new_content: string | null;
  new_binary: boolean;
}

export interface FileContentsRequest {
  path: string;
  staged: boolean;
}

export interface FileContentsBatchItem {
  path: string;
  response: FileContentsResponse | null;
  error: string | null;
}

export function getFileContentsBatch(
  requests: FileContentsRequest[],
): Promise<FileContentsBatchItem[]> {
  return invoke<FileContentsBatchItem[]>("get_file_contents_batch", { requests });
}

export function stageFile(path: string): Promise<void> {
  return invoke<void>("stage_file", { path });
}

export function stageAll(): Promise<void> {
  return invoke<void>("stage_all");
}

export function unstageFile(path: string): Promise<void> {
  return invoke<void>("unstage_file", { path });
}

export function unstageAll(): Promise<void> {
  return invoke<void>("unstage_all");
}

export interface CommitOptions {
  amend?: boolean;
}

export function commit(
  message: string,
  options?: CommitOptions,
): Promise<string> {
  return invoke<string>("commit", { message, amend: options?.amend ?? false });
}

export interface CommentIdMapping {
  key: string;
  id: string;
}

export interface SubmitReviewResponse {
  submitted_count: number;
  comment_ids: CommentIdMapping[];
}

export function submitReview(
  comments: ReviewComment[],
): Promise<SubmitReviewResponse> {
  return invoke<SubmitReviewResponse>("submit_review", { comments });
}

export interface CloneProgress {
  id: string;
  phase: "fetch" | "checkout";
  received_objects: number;
  total_objects: number;
  indexed_objects: number;
  received_bytes: number;
  checkout_current: number;
  checkout_total: number;
}

export function cloneRepo(args: {
  url: string;
  dest: string;
  id: string;
}): Promise<string> {
  return invoke<string>("clone_repo", args);
}

export function cancelClone(id: string): Promise<void> {
  return invoke<void>("cancel_clone", { id });
}

export function cleanupPath(path: string): Promise<void> {
  return invoke<void>("cleanup_path", { path });
}

export function initRepo(path: string): Promise<string> {
  return invoke<string>("init_repo", { path });
}

export function getRepoBranch(path: string): Promise<string | null> {
  return invoke<string | null>("get_repo_branch", { path });
}

export function discardFile(path: string): Promise<void> {
  return invoke<void>("discard_file", { path });
}

export function openRemoteRepo(host: string, path: string): Promise<string> {
  return invoke<string>("open_remote_repo", { host, path });
}
