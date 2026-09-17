import { webcrypto } from "node:crypto";
import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import * as Y from "yjs";
import { DocumentReviewProvider } from "./DocumentReview";
import { DocumentComments } from "./DocumentComments";
import { readCommentThreads } from "@/lib/document-comments";
import { sendReview } from "@/lib/document-reviews";
vi.mock("@/lib/users", () => ({
  listUsers: vi.fn().mockResolvedValue({ items: [], total: 0 }),
}));
vi.mock("@/lib/document-reviews", async (importOriginal) => ({
  ...(await importOriginal<object>()),
  sendReview: vi.fn().mockResolvedValue({ id: "review" }),
}));
const anchor = { quote: "Payments", start: {}, end: {} };
function surface(doc: Y.Doc, token = "alice") {
  return (
    <DocumentReviewProvider
      token={token}
      map="map"
      project="tp"
      locale="en"
      canWrite
    >
      <DocumentComments
        ydoc={doc}
        sectionId="section"
        editor={null}
        actor="Alice"
        canWrite
        locale="en"
        draft={anchor}
        onDraftConsumed={() => {}}
        onClose={() => {}}
      />
    </DocumentReviewProvider>
  );
}
beforeEach(() => {
  localStorage.clear();
  vi.clearAllMocks();
  Object.defineProperty(globalThis.crypto, "subtle", {
    value: webcrypto.subtle,
    configurable: true,
  });
});
describe("private batch reviews", () => {
  it("keeps ordinary comments instant and private drafts out of the shared document, persists and sends once", async () => {
    const doc = new Y.Doc();
    const view = render(surface(doc));
    await waitFor(() =>
      expect(
        screen
          .getByRole("button", { name: "Start review" })
          .hasAttribute("disabled"),
      ).toBe(false),
    );
    fireEvent.change(screen.getByLabelText("New comment"), {
      target: { value: "Ordinary comment" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Post comment" }));
    expect(readCommentThreads(doc)).toHaveLength(1);
    fireEvent.click(screen.getByRole("button", { name: "Start review" }));
    fireEvent.change(screen.getByLabelText("New comment"), {
      target: { value: "Private first" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Add to review" }));
    fireEvent.change(screen.getByLabelText("New comment"), {
      target: { value: "Private second" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Add to review" }));
    expect(readCommentThreads(doc)).toHaveLength(1);
    expect(sendReview).not.toHaveBeenCalled();
    view.unmount();
    render(surface(doc));
    await screen.findByText("Review in progress · 2 comments");
    fireEvent.click(screen.getByRole("button", { name: "Review and send" }));
    fireEvent.change(screen.getByLabelText("Summary"), {
      target: { value: "Payment review" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Send" }));
    await waitFor(() => expect(sendReview).toHaveBeenCalledTimes(1));
    expect(vi.mocked(sendReview).mock.calls[0]?.[2]).toMatchObject({
      title: "Payment review",
      recipients: [],
      comments: [{ text: "Private first" }, { text: "Private second" }],
    });
    await screen.findByText("Sent.");
    expect(localStorage.length).toBe(0);
  });
  it("does not reveal another credential’s private draft and retains a draft after a failed send", async () => {
    const doc = new Y.Doc();
    const view = render(surface(doc));
    await waitFor(() =>
      expect(
        screen
          .getByRole("button", { name: "Start review" })
          .hasAttribute("disabled"),
      ).toBe(false),
    );
    fireEvent.click(screen.getByRole("button", { name: "Start review" }));
    fireEvent.change(screen.getByLabelText("New comment"), {
      target: { value: "Private" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Add to review" }));
    view.unmount();
    const other = render(surface(doc, "bob"));
    await waitFor(() =>
      expect(
        screen
          .getByRole("button", { name: "Start review" })
          .hasAttribute("disabled"),
      ).toBe(false),
    );
    expect(screen.queryByText("Private")).toBeNull();
    other.unmount();
    render(surface(doc));
    await screen.findByText("Review in progress · 1 comments");
    fireEvent.click(screen.getByRole("button", { name: "Review and send" }));
    fireEvent.change(screen.getByLabelText("Summary"), {
      target: { value: "Review" },
    });
    vi.mocked(sendReview).mockRejectedValueOnce(new Error("Offline"));
    await act(async () =>
      fireEvent.click(
        screen.getByRole("button", { name: "Send" }),
      ),
    );
    expect(screen.getByRole("alert").textContent).toContain("Offline");
    expect(localStorage.length).toBe(1);
    expect(readCommentThreads(doc)).toHaveLength(0);
  });
});
