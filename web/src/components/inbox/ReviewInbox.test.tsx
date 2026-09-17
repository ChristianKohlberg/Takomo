import { useEffect, useEffectEvent } from "react";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, it, expect, vi } from "vitest";
import { ReviewInbox } from "./ReviewInbox";
import {
  listReviews,
  reviewAction,
  type DocumentReview,
} from "@/lib/document-reviews";
vi.mock("@/hooks/useLiveRefresh", () => ({
  useLiveRefresh: ({
    load,
    scope,
  }: {
    load: (signal: AbortSignal) => Promise<void>;
    scope: string;
  }) => {
    const read = useEffectEvent(load);
    useEffect(() => {
      const c = new AbortController();
      void read(c.signal);
      return () => c.abort();
    }, [scope]);
    return { refresh: vi.fn() };
  },
}));
vi.mock("@/lib/document-reviews", () => ({
  listReviews: vi.fn(),
  getReview: vi.fn(),
  reviewAction: vi.fn(),
  reviewId: () => "action-id",
}));
const review: DocumentReview = {
  id: "r",
  project: "tp",
  mindmap: "m",
  kind: "review",
  title: "Payment review",
  creator: "Alice",
  recipients: [],
  thread_ids: ["t"],
  snapshot: [
    {
      id: "t",
      sectionId: "s",
      anchor: { quote: "Payment", start: {}, end: {} },
      resolved: false,
      messages: [
        { id: "msg", author: "Alice", text: "What about retries?", created: 1 },
      ],
    },
  ],
  status: "open",
  version: 1,
  is_creator: true,
  can_work: false,
  responded: false,
  needs_me: true,
  following: true,
  source_missing: false,
};
describe("review inbox", () => {
  it("keeps thread resolution distinct from finishing a review and links both views", async () => {
    vi.mocked(listReviews).mockResolvedValue({
      items: [review],
      total: 1,
      limit: 30,
      offset: 0,
      truncated: false,
    });
    vi.mocked(reviewAction).mockResolvedValue({
      ...review,
      version: 2,
      snapshot: [{ ...review.snapshot[0]!, resolved: true }],
    });
    render(<ReviewInbox token="token" project="tp" locale="en" canWrite />);
    fireEvent.click(
      await screen.findByRole("button", { name: /Payment review/ }),
    );
    expect(
      screen.getByRole("button", { name: "Finish" }).hasAttribute("disabled"),
    ).toBe(true);
    expect(
      screen.getByRole("link", { name: "In document" }).getAttribute("href"),
    ).toContain("section=s");
    expect(
      screen.getByRole("link", { name: "In map" }).getAttribute("href"),
    ).toContain("view=map");
    fireEvent.click(screen.getByRole("button", { name: "Resolve comment" }));
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "Finish" }).hasAttribute("disabled"),
      ).toBe(false),
    );
    expect(reviewAction).toHaveBeenCalledWith("token", review, {
      request_id: "action-id",
      action: "resolve",
      thread_id: "t",
      text: undefined,
      recipients: undefined,
    });
    expect(
      screen.getByRole("button", { name: "My review input is complete" }),
    ).toBeTruthy();
    expect(screen.getByRole("button", { name: "Reopen comment" })).toBeTruthy();
  });
});
