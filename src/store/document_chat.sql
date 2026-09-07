-- Document conversations use the otherwise-unaddressable empty node anchor.
-- Keeping the mindmap FK on agent_conversations preserves deletion cascades.
CREATE TABLE IF NOT EXISTS document_agent_jobs (
  job TEXT PRIMARY KEY REFERENCES agent_jobs(id) ON DELETE CASCADE,
  action TEXT NOT NULL,
  section_ids TEXT NOT NULL,
  whole_document INTEGER NOT NULL CHECK(whole_document IN (0,1))
);
