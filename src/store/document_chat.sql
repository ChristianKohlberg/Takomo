-- Document conversations use the otherwise-unaddressable empty node anchor.
-- Keeping the mindmap FK on agent_conversations preserves deletion cascades.
CREATE TABLE IF NOT EXISTS document_agent_jobs (
  job TEXT PRIMARY KEY REFERENCES agent_jobs(id) ON DELETE CASCADE,
  action TEXT NOT NULL,
  section_ids TEXT NOT NULL,
  whole_document INTEGER NOT NULL CHECK(whole_document IN (0,1))
);

CREATE TABLE IF NOT EXISTS document_workspace_jobs (
  job TEXT PRIMARY KEY REFERENCES document_agent_jobs(job) ON DELETE CASCADE,
  context TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS document_agent_settings (
  mindmap TEXT PRIMARY KEY REFERENCES mindmaps(id) ON DELETE CASCADE,
  pinned_section_ids TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS document_thread_migrations (
  job TEXT PRIMARY KEY REFERENCES document_workspace_jobs(job) ON DELETE CASCADE,
  metadata TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS document_thread_profiles (
  conversation_id TEXT PRIMARY KEY REFERENCES agent_conversations(id) ON DELETE CASCADE,
  thread_id TEXT NOT NULL
);
