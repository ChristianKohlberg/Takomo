CREATE TABLE IF NOT EXISTS document_reviews (
 id TEXT PRIMARY KEY,
 project TEXT NOT NULL REFERENCES projects(id),
 mindmap TEXT NOT NULL REFERENCES mindmaps(id) ON DELETE CASCADE,
 kind TEXT NOT NULL CHECK(kind IN ('review','question','change','mention')),
 title TEXT NOT NULL,
 creator TEXT NOT NULL,
 creator_key TEXT NOT NULL,
 recipients TEXT NOT NULL,
 thread_ids TEXT NOT NULL,
 snapshot TEXT NOT NULL,
 status TEXT NOT NULL DEFAULT 'open' CHECK(status IN ('open','in_progress','ready','closed')),
 version INTEGER NOT NULL DEFAULT 1,
 request_id TEXT NOT NULL,
 intent TEXT NOT NULL,
 created_at INTEGER NOT NULL,
 updated_at INTEGER NOT NULL,
 UNIQUE(mindmap,creator_key,request_id)
);
CREATE INDEX IF NOT EXISTS document_reviews_project ON document_reviews(project,updated_at);
CREATE TABLE IF NOT EXISTS document_review_responses (
 review TEXT NOT NULL REFERENCES document_reviews(id) ON DELETE CASCADE,
 identity TEXT NOT NULL,
 completed INTEGER NOT NULL DEFAULT 0,
 PRIMARY KEY(review,identity)
);
CREATE TABLE IF NOT EXISTS document_review_actions (
 review TEXT NOT NULL REFERENCES document_reviews(id) ON DELETE CASCADE,
 identity TEXT NOT NULL,
 request_id TEXT NOT NULL,
 intent TEXT NOT NULL,
 PRIMARY KEY(review,identity,request_id)
);
