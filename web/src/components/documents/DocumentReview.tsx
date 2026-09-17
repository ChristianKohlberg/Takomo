import {
  Dialog,
  DialogContent,
  DialogTitle,
  DialogDescription,
} from "@/components/ui/dialog";
import {
  createContext,
  useContext,
  useEffect,
  useState,
  type ReactNode,
} from "react";
import { Button } from "@/components/ui/button";
import { listUsers, type User } from "@/lib/users";
import {
  reviewId,
  sendReview,
  type ReviewDraft,
  type ReviewKind,
} from "@/lib/document-reviews";
import type { CommentAnchor, CommentThread } from "@/lib/document-comments";
import type { Locale } from "@/lib/i18n";
interface ReviewContextValue {
  active: boolean;
  add: (section: string, anchor: CommentAnchor, text: string) => void;
  draft: ReviewDraft | null;
  route: (thread: CommentThread, kind: ReviewKind) => void;
}
const ReviewContext = createContext<ReviewContextValue | null>(null);
export const useDocumentReview = () => useContext(ReviewContext);
const field =
  "w-full min-w-0 rounded border border-border bg-background p-2 text-sm";
export function ReviewPeople({
  token,
  project,
  value,
  onChange,
  locale,
  single = false,
}: {
  token: string;
  project: string;
  value: string[];
  onChange: (v: string[]) => void;
  locale: Locale;
  single?: boolean;
}) {
  const [users, setUsers] = useState<User[]>([]),
    [query, setQuery] = useState(""),
    [error, setError] = useState(""),
    [total, setTotal] = useState(0);
  useEffect(() => {
    let live = true;
    const timer = setTimeout(() => {
      void listUsers(token, { project, q: query, limit: 100 })
        .then((p) => {
          if (live) {
            setUsers(p.items);
            setTotal(p.total);
            setError("");
          }
        })
        .catch((e) => {
          if (live) setError(String(e));
        });
    }, 200);
    return () => {
      live = false;
      clearTimeout(timer);
    };
  }, [token, project, query]);
  return (
    <fieldset className="space-y-2">
      <legend className="text-sm font-medium">
        {locale === "de" ? "Empfänger" : "Recipients"}
        {single ? "" : ` (${locale === "de" ? "optional" : "optional"})`}
      </legend>
      <input
        className={field}
        aria-label={locale === "de" ? "Personen suchen" : "Find people"}
        placeholder={locale === "de" ? "Personen suchen" : "Find people"}
        value={query}
        onChange={(e) => setQuery(e.target.value)}
      />
      {value.length > 0 && (
        <button
          type="button"
          className="text-sm underline"
          onClick={() => onChange([])}
        >
          {locale === "de" ? "Auswahl löschen" : "Clear selection"} (
          {value.length})
        </button>
      )}
      <div className="max-h-36 overflow-y-auto">
        {users
          .filter((u) => !u.disabled)
          .map((u) => (
            <label className="flex items-center gap-2 py-1 text-sm" key={u.id}>
              <input
                type="checkbox"
                checked={value.includes(u.id)}
                onChange={(e) =>
                  onChange(
                    e.target.checked
                      ? single
                        ? [u.id]
                        : [...value, u.id]
                      : value.filter((id) => id !== u.id),
                  )
                }
              />
              {u.label}
            </label>
          ))}
      </div>
      {total > users.length && (
        <p className="text-xs text-muted-foreground">
          {locale === "de"
            ? "Suche eingrenzen, um weitere Personen zu finden."
            : "Narrow your search to find more people."}
        </p>
      )}
      {error && <p role="alert">{error}</p>}
    </fieldset>
  );
}
export function DocumentReviewProvider({
  token,
  map,
  project,
  locale,
  canWrite,
  children,
}: {
  token: string;
  map: string;
  project: string;
  locale: Locale;
  canWrite: boolean;
  children: ReactNode;
}) {
  const de = locale === "de";
  const [key, setKey] = useState(""),
    [draft, setDraft] = useState<ReviewDraft | null>(null),
    [open, setOpen] = useState(false),
    [error, setError] = useState(""),
    [busy, setBusy] = useState(false),
    [sent, setSent] = useState(false);
  const [routing, setRouting] = useState<{
    thread: CommentThread;
    kind: ReviewKind;
    draft: ReviewDraft;
  } | null>(null);
  useEffect(() => {
    let live = true;
    setKey("");
    setDraft(null);
    setRouting(null);
    setOpen(false);
    void crypto.subtle
      .digest("SHA-256", new TextEncoder().encode(token))
      .then((hash) => {
        if (!live) return;
        const k = `takomo.review.${Array.from(new Uint8Array(hash), (x) => x.toString(16).padStart(2, "0")).join("")}.${map}`;
        try {
          const raw = localStorage.getItem(k);
          setDraft(raw ? (JSON.parse(raw) as ReviewDraft) : null);
          setKey(k);
        } catch {
          setError(
            de
              ? "Privater Entwurf kann nicht gespeichert werden."
              : "Private draft storage is unavailable.",
          );
        }
      });
    return () => {
      live = false;
    };
  }, [token, map, de]);
  useEffect(() => {
    const sync = (event: StorageEvent) => {
      if (event.key !== key) return;
      try {
        setDraft(
          event.newValue ? (JSON.parse(event.newValue) as ReviewDraft) : null,
        );
      } catch {
        setError(
          de
            ? "Entwurf kann nicht gelesen werden."
            : "Could not read the draft.",
        );
      }
    };
    window.addEventListener("storage", sync);
    return () => window.removeEventListener("storage", sync);
  }, [key, de]);
  const save = (next: ReviewDraft | null) => {
    if (!key)
      throw Error(
        de ? "Entwurf ist noch nicht bereit." : "Draft storage is not ready.",
      );
    if (localStorage.getItem(key) !== (draft ? JSON.stringify(draft) : null)) {
      const stored = localStorage.getItem(key);
      setDraft(stored ? (JSON.parse(stored) as ReviewDraft) : null);
      throw Error(
        de
          ? "Der Entwurf wurde in einem anderen Tab geändert. Bitte erneut versuchen."
          : "The draft changed in another tab. Please retry.",
      );
    }
    if (next) localStorage.setItem(key, JSON.stringify(next));
    else localStorage.removeItem(key);
    setDraft(next);
    setSent(false);
  };
  const attempt = (fn: () => void) => {
    try {
      fn();
      setError("");
    } catch (e) {
      setError(String(e));
    }
  };
  const activeDraft = routing?.draft ?? draft;
  const update = (next: ReviewDraft) => {
    if (routing) setRouting({ ...routing, draft: next });
    else attempt(() => save(next));
  };
  const submit = async () => {
    if (!activeDraft) return;
    setBusy(true);
    setError("");
    try {
      await sendReview(token, map, {
        ...activeDraft,
        kind: routing?.kind ?? "review",
        ...(routing ? { thread_ids: [routing.thread.id] } : {}),
      });
      if (!routing) save(null);
      setRouting(null);
      setOpen(false);
      setSent(true);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <ReviewContext.Provider
      value={{
        active: !!draft,
        draft,
        add: (section, anchor, text) => {
          if (!draft) throw Error("No active review");
          if (draft.comments.length >= 100)
            throw Error(
              de
                ? "Maximal 100 Kommentare."
                : "At most 100 comments per review.",
            );
          save({
            ...draft,
            comments: [
              ...draft.comments,
              {
                id: reviewId(),
                section_id: section,
                anchor,
                text: text.trim(),
              },
            ],
          });
        },
        route: (thread, kind) => {
          setError("");
          setRouting({
            thread,
            kind,
            draft: {
              request_id: reviewId(),
              title: thread.messages[0]?.text.slice(0, 300) ?? "",
              comments: [],
              recipients: [],
            },
          });
          setOpen(true);
        },
      }}
    >
      {canWrite && token && (
        <div className="flex shrink-0 flex-wrap items-center gap-3 border-b border-border-soft bg-white px-4 py-2 text-sm dark:bg-card">
          {draft ? (
            <>
              <span>
                {de ? "Review läuft" : "Review in progress"} ·{" "}
                {draft.comments.length} {de ? "Kommentare" : "comments"}
              </span>
              <span className="text-xs text-muted-foreground">
                {de
                  ? "Privater Entwurf · nur in diesem Browser"
                  : "Private draft · this browser only"}
              </span>
              <Button
                size="sm"
                variant="outline"
                onClick={() => {
                  setRouting(null);
                  setOpen(true);
                }}
              >
                {de ? "Review ansehen und senden" : "Review and send"}
              </Button>
            </>
          ) : (
            <Button
              size="sm"
              variant="ghost"
              disabled={!key}
              onClick={() =>
                attempt(() =>
                  save({
                    request_id: reviewId(),
                    title: "",
                    comments: [],
                    recipients: [],
                  }),
                )
              }
            >
              {de ? "Review starten" : "Start review"}
            </Button>
          )}
          {sent && (
            <span role="status">
              {de ? "Gesendet." : "Sent."}{" "}
              <a className="underline" href="/inbox?tab=reviews">
                {de ? "Im Posteingang öffnen" : "Open in inbox"}
              </a>
            </span>
          )}
          {error && !open && <span role="alert">{error}</span>}
        </div>
      )}
      {children}
      {open && activeDraft && (
        <Dialog
          open
          onOpenChange={(value) => {
            if (!busy) setOpen(value);
          }}
        >
          <DialogContent
            showCloseButton={!busy}
            className="max-h-[90dvh] overflow-y-auto sm:max-w-xl"
          >
            <fieldset disabled={busy} className="min-w-0">
              <DialogDescription className="sr-only">
                {de
                  ? "Kommentare sammeln und gemeinsam senden."
                  : "Collect comments and send them together."}
              </DialogDescription>
              <DialogTitle className="mb-3 text-lg font-semibold">
                {routing
                  ? {
                      question: de ? "Antwort anfordern" : "Request an answer",
                      change: de ? "Änderung anfordern" : "Request a change",
                      mention: de
                        ? "Jemanden benachrichtigen"
                        : "Notify someone",
                      review: "Review",
                    }[routing.kind]
                  : de
                    ? "Review senden"
                    : "Send review"}
              </DialogTitle>
              <label className="block text-sm">
                {de ? "Zusammenfassung" : "Summary"}
                <textarea
                  autoFocus
                  className={field}
                  maxLength={300}
                  rows={2}
                  value={activeDraft.title}
                  onChange={(e) =>
                    update({ ...activeDraft, title: e.target.value })
                  }
                />
              </label>
              <div className="my-3 space-y-2">
                {routing ? (
                  <blockquote className="border-l-2 border-border pl-3 text-sm">
                    {routing.thread.anchor.quote}
                  </blockquote>
                ) : (
                  activeDraft.comments.map((c) => (
                    <article
                      className="rounded border border-border-soft p-3 text-sm"
                      key={c.id}
                    >
                      <blockquote className="mb-2 text-muted-foreground">
                        {c.anchor.quote}
                      </blockquote>
                      <textarea
                        aria-label={
                          de ? "Entwurf bearbeiten" : "Edit draft comment"
                        }
                        className={field}
                        maxLength={5000}
                        value={c.text}
                        onChange={(e) =>
                          update({
                            ...activeDraft,
                            comments: activeDraft.comments.map((item) =>
                              item.id === c.id
                                ? { ...item, text: e.target.value }
                                : item,
                            ),
                          })
                        }
                      />
                      <button
                        type="button"
                        className="mt-1 underline"
                        onClick={() =>
                          update({
                            ...activeDraft,
                            comments: activeDraft.comments.filter(
                              (item) => item.id !== c.id,
                            ),
                          })
                        }
                      >
                        {de ? "Entfernen" : "Remove"}
                      </button>
                    </article>
                  ))
                )}
              </div>
              <ReviewPeople
                token={token}
                project={project}
                locale={locale}
                value={activeDraft.recipients}
                single={routing?.kind === "change"}
                onChange={(recipients) =>
                  update({ ...activeDraft, recipients })
                }
              />
              <p className="my-3 text-xs text-muted-foreground">
                {routing?.kind === "change"
                  ? de
                    ? "Eine Person übernimmt die Änderung. Du schließt sie nach Prüfung ab."
                    : "One person owns the change. You close it after checking the result."
                  : routing?.kind === "mention"
                    ? de
                      ? "Benachrichtigung ohne Arbeitsauftrag."
                      : "A notification without assigning work."
                    : de
                      ? "Ohne Empfänger erscheint dies in der gemeinsamen Projektwarteschlange."
                      : "Without recipients, this goes to the project’s shared review queue."}
              </p>
              {error && (
                <p role="alert" className="my-2 text-sm">
                  {error}
                </p>
              )}
              <div className="flex flex-wrap gap-2">
                <Button
                  disabled={
                    busy ||
                    !activeDraft.title.trim() ||
                    (!routing &&
                      (!activeDraft.comments.length ||
                        activeDraft.comments.some((c) => !c.text.trim()))) ||
                    ((routing?.kind === "change" ||
                      routing?.kind === "mention") &&
                      !activeDraft.recipients.length)
                  }
                  onClick={() => void submit()}
                >
                  {busy
                    ? de
                      ? "Wird gesendet…"
                      : "Sending…"
                    : de
                      ? "Senden"
                      : "Send"}
                </Button>
                <Button
                  variant="outline"
                  disabled={busy}
                  onClick={() => setOpen(false)}
                >
                  {de ? "Weiter kommentieren" : "Keep commenting"}
                </Button>
                {!routing && (
                  <Button
                    variant="ghost"
                    disabled={busy}
                    onClick={() => {
                      if (
                        window.confirm(
                          de
                            ? "Review-Entwurf mit allen Kommentaren verwerfen?"
                            : "Discard this review draft and all its comments?",
                        )
                      )
                        attempt(() => {
                          save(null);
                          setOpen(false);
                        });
                    }}
                  >
                    {de ? "Entwurf verwerfen" : "Discard draft"}
                  </Button>
                )}
              </div>
            </fieldset>
          </DialogContent>
        </Dialog>
      )}
    </ReviewContext.Provider>
  );
}
