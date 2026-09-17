import { api } from "./api";
import type { CommentAnchor, CommentThread } from "./document-comments";
import type { UserRef } from "./users";
export interface ReviewComment {
  id: string;
  section_id: string;
  anchor: CommentAnchor;
  text: string;
}
export type ReviewKind = "review" | "question" | "change" | "mention";
export interface ReviewDraft {
  request_id: string;
  title: string;
  comments: ReviewComment[];
  recipients: string[];
}
export interface DocumentReview {
  id: string;
  project: string;
  mindmap: string;
  kind: ReviewKind;
  title: string;
  creator: string;
  recipients: UserRef[];
  thread_ids: string[];
  snapshot: CommentThread[];
  status: "open" | "in_progress" | "ready" | "closed";
  version: number;
  is_creator: boolean;
  can_work: boolean;
  responded: boolean;
  needs_me: boolean;
  following: boolean;
  source_missing: boolean;
}
export interface ReviewsPage {
  items: DocumentReview[];
  total: number;
  limit: number;
  offset: number;
  truncated: boolean;
}
export const reviewId = () => crypto.randomUUID();
const json = { "Content-Type": "application/json" };
export function sendReview(
  token: string,
  map: string,
  body: ReviewDraft & { kind?: ReviewKind; thread_ids?: string[] },
) {
  return api<DocumentReview>(
    token,
    `/mindmaps/${encodeURIComponent(map)}/reviews`,
    { method: "POST", headers: json, body: JSON.stringify(body) },
  );
}
export function listReviews(
  token: string,
  project: string,
  queue: string,
  offset = 0,
  signal?: AbortSignal,
) {
  const q = new URLSearchParams({ queue, offset: String(offset), limit: "30" });
  if (project) q.set("project", project);
  return api<ReviewsPage>(token, `/document-reviews?${q}`, { signal });
}
export function getReview(token: string, id: string, signal?: AbortSignal) {
  return api<DocumentReview>(
    token,
    `/document-reviews/${encodeURIComponent(id)}`,
    { signal },
  );
}
export function reviewAction(
  token: string,
  review: DocumentReview,
  body: {
    request_id: string;
    action: string;
    thread_id?: string;
    text?: string;
    recipients?: string[];
  },
) {
  return api<DocumentReview>(
    token,
    `/document-reviews/${encodeURIComponent(review.id)}/actions`,
    {
      method: "POST",
      headers: json,
      body: JSON.stringify({ ...body, version: review.version }),
    },
  );
}
