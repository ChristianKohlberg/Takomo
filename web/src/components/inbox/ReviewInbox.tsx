import { useRef, useState } from "react";
import { Button } from "@/components/ui/button";
import { ReviewPeople } from "@/components/documents/DocumentReview";
import { useLiveRefresh } from "@/hooks/useLiveRefresh";
import {
  getReview,
  listReviews,
  reviewAction,
  reviewId,
  type DocumentReview,
  type ReviewsPage,
} from "@/lib/document-reviews";
import type { Locale } from "@/lib/i18n";
import { specificationLink } from "@/lib/specification-url";
export function ReviewInbox({
  token,
  project,
  locale,
  canWrite,
  admin = false,
}: {
  token: string;
  project: string;
  locale: Locale;
  canWrite: boolean;
  admin?: boolean;
}) {
  const de = locale === "de",
    [queue, setQueue] = useState("needs_me"),
    [offset, setOffset] = useState(0),
    [page, setPage] = useState<ReviewsPage | null>(null),
    [selected, setSelected] = useState<DocumentReview | null>(null),
    [error, setError] = useState(""),
    [busy, setBusy] = useState(false);
  const [replies, setReplies] = useState<Record<string, string>>({}),
    [assign, setAssign] = useState<string[] | null>(null);
  const pending = useRef<{
    intent: string;
    id: string;
    version: number;
  } | null>(null);
  const { refresh } = useLiveRefresh({
    token,
    project,
    scope: `reviews:${queue}:${offset}`,
    topics: ["inbox", "document"],
    paused: busy,
    onError: (e) => setError(String(e)),
    load: async (signal) => {
      const p = await listReviews(token, project, queue, offset, signal);
      if (!signal.aborted) setPage(p);
      if (selected) {
        const next = await getReview(token, selected.id, signal);
        if (!signal.aborted)
          setSelected((current) => (current?.id === next.id ? next : current));
      }
    },
  });
  const act = async (
    action: string,
    thread_id?: string,
    text?: string,
    recipients?: string[],
  ) => {
    if (!selected || busy) return;
    setBusy(true);
    setError("");
    const intent = JSON.stringify([
      selected.id,
      action,
      thread_id,
      text,
      recipients,
    ]);
    if (pending.current?.intent !== intent)
      pending.current = { intent, id: reviewId(), version: selected.version };
    try {
      const v = await reviewAction(
        token,
        { ...selected, version: pending.current.version },
        { request_id: pending.current.id, action, thread_id, text, recipients },
      );
      setSelected((current) => (current?.id === v.id ? v : current));
      pending.current = null;
      if (thread_id) setReplies((r) => ({ ...r, [thread_id]: "" }));
      setAssign(null);
      refresh();
    } catch (e) {
      setError(String(e));
      if ((e as { status?: number }).status) {
        pending.current = null;
        try {
          const fresh = await getReview(token, selected.id);
          setSelected((current) =>
            current?.id === fresh.id ? fresh : current,
          );
        } catch {
          /* retain original error */
        }
      }
    } finally {
      setBusy(false);
    }
  };
  const kind = (v: DocumentReview) =>
    ({
      review: "Review",
      question: de ? "Frage" : "Question",
      change: de ? "Änderung" : "Change request",
      mention: de ? "Benachrichtigung" : "Notification",
    })[v.kind];
  const status = (v: DocumentReview) =>
    ({
      open: de ? "Offen" : "Open",
      in_progress: de ? "In Arbeit" : "In progress",
      ready: de ? "Bereit zur Prüfung" : "Ready for review",
      closed: de ? "Abgeschlossen" : "Closed",
    })[v.status];
  return (
    <section className="flex min-h-0 flex-1 flex-col" aria-label="Reviews">
      <nav
        className="flex shrink-0 flex-wrap gap-2 border-b border-border-soft px-4 py-2"
        aria-label={de ? "Review-Ordner" : "Review folders"}
      >
        {[
          ["needs_me", de ? "Braucht mich" : "Needs me"],
          ["following", de ? "Verfolge ich" : "Following"],
          ["mine", de ? "Meine Anfragen" : "My requests"],
          ["shared", de ? "Gemeinsam" : "Shared"],
          ["all", de ? "Alle" : "All"],
        ].map(([id, label]) => (
          <Button
            key={id}
            size="sm"
            variant={queue === id ? "secondary" : "ghost"}
            onClick={() => {
              setQueue(id!);
              setOffset(0);
              setSelected(null);
              setPage(null);
            }}
          >
            {label}
          </Button>
        ))}
        <Button variant="ghost" size="sm" onClick={() => refresh()}>
          {de ? "Aktualisieren" : "Refresh"}
        </Button>
      </nav>
      {error && (
        <p role="alert" className="p-3 text-sm">
          {error}
        </p>
      )}
      <div className="flex min-h-0 flex-1 overflow-hidden">
        <aside
          className={`${selected ? "hidden md:block" : "block"} w-full shrink-0 overflow-y-auto border-r border-border-soft md:w-80`}
        >
          {!page ? (
            <p className="p-4">{de ? "Lädt…" : "Loading…"}</p>
          ) : (
            <>
              <p className="px-4 py-2 text-xs text-muted-foreground">
                {page.total} {de ? "Einträge" : "items"}
              </p>
              {page.items.length === 0 && (
                <p className="p-4 text-sm text-muted-foreground">
                  {de ? "Hier wartet nichts." : "Nothing waiting here."}
                </p>
              )}
              {page.items.map((v) => (
                <button
                  key={v.id}
                  className={`block w-full border-b border-border-soft p-4 text-left hover:bg-muted ${selected?.id === v.id ? "bg-muted" : ""}`}
                  onClick={() => {
                    setSelected(v);
                    setError("");
                    setAssign(null);
                  }}
                >
                  <span className="text-xs text-muted-foreground">
                    {kind(v)} · {status(v)}
                  </span>
                  <span className="my-1 block font-medium break-words">
                    {v.title}
                  </span>
                  <span className="block text-xs text-muted-foreground">
                    {v.creator} · {v.snapshot.filter((t) => t.resolved).length}/
                    {v.snapshot.length} {de ? "erledigt" : "resolved"}
                  </span>
                  <span className="block text-xs text-muted-foreground">
                    {v.recipients.map((p) => p.label).join(", ") ||
                      (de
                        ? "Gemeinsame Projektwarteschlange"
                        : "Shared project queue")}
                  </span>
                </button>
              ))}
              <div className="flex gap-2 p-3">
                <Button
                  size="sm"
                  variant="ghost"
                  disabled={!offset}
                  onClick={() => setOffset(Math.max(0, offset - 30))}
                >
                  {de ? "Zurück" : "Previous"}
                </Button>
                <Button
                  size="sm"
                  variant="ghost"
                  disabled={!page.truncated}
                  onClick={() => setOffset(offset + 30)}
                >
                  {de ? "Weiter" : "Next"}
                </Button>
              </div>
            </>
          )}
        </aside>
        <main
          className={`${selected ? "block" : "hidden md:block"} min-w-0 flex-1 overflow-y-auto p-4 md:p-6`}
        >
          {selected ? (
            <>
              <Button
                size="sm"
                variant="ghost"
                className="mb-3 md:hidden"
                onClick={() => setSelected(null)}
              >
                {de ? "Zur Liste" : "Back to list"}
              </Button>
              <p className="text-sm text-muted-foreground">
                {kind(selected)} · {status(selected)}
              </p>
              <h2 className="my-2 text-xl font-semibold break-words">
                {selected.title}
              </h2>
              <p className="text-sm">
                {selected.creator} →{" "}
                {selected.recipients.map((p) => p.label).join(", ") ||
                  (de
                    ? "Gemeinsame Projektwarteschlange"
                    : "Shared project queue")}
              </p>
              {selected.source_missing && (
                <p
                  role="status"
                  className="my-3 rounded border border-border p-3 text-sm"
                >
                  {de
                    ? "Quellkommentar entfernt. Gespeicherter Verlauf; Antworten sind nicht mehr möglich."
                    : "Source comment removed. Showing saved history; replies are unavailable."}
                </p>
              )}
              <div className="my-4 flex flex-wrap gap-2">
                {canWrite && !selected.source_missing && (
                  <>
                    {selected.status !== "closed" &&
                      !selected.responded &&
                      selected.kind === "review" && (
                        <Button
                          size="sm"
                          disabled={busy}
                          onClick={() => void act("reviewed")}
                        >
                          {de
                            ? "Mein Review erledigt"
                            : "My review input is complete"}
                        </Button>
                      )}
                    {selected.kind === "change" &&
                      selected.status === "open" &&
                      selected.can_work && (
                        <Button
                          size="sm"
                          disabled={busy}
                          onClick={() => void act("start")}
                        >
                          {de ? "Arbeit beginnen" : "Start work"}
                        </Button>
                      )}
                    {selected.kind === "change" &&
                      selected.status === "in_progress" &&
                      selected.can_work && (
                        <Button
                          size="sm"
                          disabled={busy}
                          onClick={() => void act("ready")}
                        >
                          {de ? "Zur Prüfung bereit" : "Ready for review"}
                        </Button>
                      )}
                    {(selected.is_creator || admin) && (
                      <>
                        {selected.status === "closed" ? (
                          <Button
                            size="sm"
                            disabled={busy}
                            onClick={() => void act("reopen")}
                          >
                            {de ? "Wieder öffnen" : "Reopen"}
                          </Button>
                        ) : (
                          <>
                            {selected.kind !== "question" &&
                              (selected.kind !== "change" ||
                                selected.status === "ready") && (
                                <Button
                                  size="sm"
                                  disabled={
                                    busy ||
                                    (selected.kind === "review" &&
                                      selected.snapshot.some(
                                        (t) => !t.resolved,
                                      ))
                                  }
                                  onClick={() => void act("close")}
                                >
                                  {de ? "Abschließen" : "Finish"}
                                </Button>
                              )}
                            <Button
                              size="sm"
                              variant="outline"
                              disabled={busy}
                              onClick={() =>
                                setAssign(selected.recipients.map((p) => p.id))
                              }
                            >
                              {de ? "Empfänger ändern" : "Change recipients"}
                            </Button>
                          </>
                        )}
                      </>
                    )}
                  </>
                )}
              </div>
              {assign && (
                <div className="my-3 rounded border border-border p-3">
                  <ReviewPeople
                    token={token}
                    project={selected.project}
                    locale={locale}
                    value={assign}
                    onChange={setAssign}
                    single={selected.kind === "change"}
                  />
                  <Button
                    size="sm"
                    disabled={
                      busy ||
                      (selected.kind === "change" && assign.length !== 1)
                    }
                    onClick={() =>
                      void act("assign", undefined, undefined, assign)
                    }
                  >
                    {de ? "Speichern" : "Save"}
                  </Button>
                  <Button
                    size="sm"
                    variant="ghost"
                    onClick={() => setAssign(null)}
                  >
                    {de ? "Abbrechen" : "Cancel"}
                  </Button>
                </div>
              )}
              {selected.snapshot.map((thread) => (
                <article
                  key={thread.id}
                  className="mb-4 rounded border border-border-soft p-4"
                >
                  <div className="mb-2 flex flex-wrap gap-3 text-xs">
                    <span>
                      {thread.resolved
                        ? de
                          ? "Erledigt"
                          : "Resolved"
                        : de
                          ? "Offen"
                          : "Open"}
                    </span>
                    <a
                      className="underline"
                      href={specificationLink(
                        selected.project,
                        "document",
                        thread.sectionId,
                      )}
                    >
                      {de ? "Im Dokument" : "In document"}
                    </a>
                    <a
                      className="underline"
                      href={specificationLink(
                        selected.project,
                        "map",
                        thread.sectionId,
                      )}
                    >
                      {de ? "In der Map" : "In map"}
                    </a>
                  </div>
                  <blockquote className="border-l-2 border-border pl-3 text-sm break-words">
                    {thread.anchor.quote}
                  </blockquote>
                  {thread.messages.map((m) => (
                    <div key={m.id} className="mt-3 text-sm">
                      <span className="font-medium">{m.author}</span>
                      <p className="whitespace-pre-wrap break-words">
                        {m.text}
                      </p>
                    </div>
                  ))}
                  {canWrite && !selected.source_missing && (
                    <>
                      <Button
                        variant="ghost"
                        size="sm"
                        disabled={busy || selected.status === "closed"}
                        onClick={() =>
                          void act(
                            thread.resolved ? "unresolve" : "resolve",
                            thread.id,
                          )
                        }
                      >
                        {thread.resolved
                          ? de
                            ? "Wieder öffnen"
                            : "Reopen comment"
                          : de
                            ? "Kommentar erledigen"
                            : "Resolve comment"}
                      </Button>
                      <form
                        className="mt-2 space-y-2"
                        onSubmit={(e) => {
                          e.preventDefault();
                          void act(
                            "reply",
                            thread.id,
                            replies[thread.id]?.trim(),
                          );
                        }}
                      >
                        <textarea
                          aria-label={de ? "Antwort" : "Reply"}
                          disabled={busy}
                          maxLength={5000}
                          rows={2}
                          className="w-full rounded border border-border bg-background p-2 text-sm"
                          value={replies[thread.id] ?? ""}
                          onChange={(e) =>
                            setReplies((r) => ({
                              ...r,
                              [thread.id]: e.target.value,
                            }))
                          }
                        />
                        <div className="flex flex-wrap gap-2">
                          <Button
                            size="sm"
                            disabled={busy || !replies[thread.id]?.trim()}
                          >
                            {de ? "Antworten" : "Reply"}
                          </Button>
                          {selected.kind === "question" &&
                            selected.status !== "closed" && (
                              <Button
                                type="button"
                                size="sm"
                                variant="outline"
                                disabled={busy || !replies[thread.id]?.trim()}
                                onClick={() =>
                                  void act(
                                    "close",
                                    thread.id,
                                    replies[thread.id]?.trim(),
                                  )
                                }
                              >
                                {de
                                  ? "Antwort senden und Frage abschließen"
                                  : "Answer and close question"}
                              </Button>
                            )}
                        </div>
                      </form>
                    </>
                  )}
                </article>
              ))}
              <p className="text-xs text-muted-foreground">
                {de
                  ? "Kommentare erledigen und Reviews abschließen erteilt keine Dokumentfreigabe."
                  : "Resolving comments and finishing reviews does not approve the document."}
              </p>
            </>
          ) : (
            <p className="text-sm text-muted-foreground">
              {de
                ? "Review auswählen, um die Kommentare zu lesen."
                : "Select a review to read its comments."}
            </p>
          )}
        </main>
      </div>
    </section>
  );
}
