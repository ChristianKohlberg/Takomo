-- Stable document relations are independent of ticket hierarchy and dependencies.
CREATE TABLE IF NOT EXISTS ticket_document_links (
 id TEXT PRIMARY KEY,
 ticket TEXT NOT NULL REFERENCES tickets(id) ON DELETE CASCADE,
 project TEXT NOT NULL,
 mindmap TEXT NOT NULL,
 section_id TEXT NOT NULL,
 title TEXT NOT NULL,
 section_version TEXT NOT NULL,
 relation TEXT NOT NULL CHECK(relation IN ('source','related')),
 provenance TEXT NOT NULL CHECK(provenance IN ('direct','manual','automatic')),
 state TEXT NOT NULL CHECK(state IN ('suggested','accepted','removed')),
 is_primary INTEGER NOT NULL DEFAULT 0,
 reason TEXT NOT NULL,
 quote TEXT NOT NULL DEFAULT '',
 ticket_revision TEXT NOT NULL,
 job TEXT REFERENCES agent_jobs(id) ON DELETE SET NULL,
 created_by TEXT NOT NULL,
 created_at INTEGER NOT NULL,
 updated_at INTEGER NOT NULL,
 reviewed_by TEXT,
 reviewed_at INTEGER
);
CREATE UNIQUE INDEX IF NOT EXISTS ticket_document_primary ON ticket_document_links(ticket) WHERE state='accepted' AND is_primary=1;
CREATE INDEX IF NOT EXISTS ticket_document_target ON ticket_document_links(project,mindmap,section_id,state);
CREATE INDEX IF NOT EXISTS ticket_document_subject ON ticket_document_links(ticket,state);
CREATE TABLE IF NOT EXISTS ticket_document_settings (
 project TEXT PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
 mode TEXT NOT NULL CHECK(mode IN ('suggest','auto_apply_clear'))
);
CREATE TABLE IF NOT EXISTS ticket_document_jobs (
 job TEXT PRIMARY KEY REFERENCES agent_jobs(id) ON DELETE CASCADE,
 ticket TEXT NOT NULL REFERENCES tickets(id) ON DELETE CASCADE,
 ticket_revision TEXT NOT NULL,
 document_revision TEXT NOT NULL,
 proposal TEXT,
 reconsider INTEGER NOT NULL DEFAULT 0,
 stale INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS ticket_document_job_subject ON ticket_document_jobs(ticket);
CREATE TABLE IF NOT EXISTS ticket_document_pending (
 ticket TEXT PRIMARY KEY REFERENCES tickets(id) ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS ticket_document_state (
 ticket TEXT PRIMARY KEY REFERENCES tickets(id) ON DELETE CASCADE,
 revision TEXT NOT NULL,
 status TEXT NOT NULL,
 error TEXT,
 request_id TEXT
);
-- Only material edits enqueue, never a read, poll, status update or claim.
CREATE TRIGGER IF NOT EXISTS ticket_document_created AFTER INSERT ON tickets BEGIN
 INSERT OR IGNORE INTO ticket_document_pending(ticket) VALUES(NEW.id);
END;
CREATE TRIGGER IF NOT EXISTS ticket_document_edited AFTER UPDATE OF title,body,project,parent ON tickets
 WHEN OLD.title<>NEW.title OR OLD.body<>NEW.body OR OLD.project<>NEW.project OR OLD.parent IS NOT NEW.parent BEGIN
 INSERT OR IGNORE INTO ticket_document_pending(ticket) VALUES(NEW.id);
END;
-- Classification conversations deliberately do not reuse a bug's unique ticket
-- conversation. Remove their source snapshots with the originating ticket.
CREATE TRIGGER IF NOT EXISTS ticket_document_deleted BEFORE DELETE ON tickets BEGIN
 DELETE FROM agent_conversations WHERE id IN (
  SELECT j.conversation_id FROM agent_jobs j JOIN ticket_document_jobs d ON d.job=j.id WHERE d.ticket=OLD.id
 );
END;
