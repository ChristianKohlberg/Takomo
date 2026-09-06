-- On-demand agent conversations are deliberately independent of tickets.
CREATE TABLE IF NOT EXISTS agent_conversations (
  id TEXT PRIMARY KEY,
  mindmap TEXT NOT NULL REFERENCES mindmaps(id) ON DELETE CASCADE,
  node TEXT NOT NULL,
  project TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  service_id TEXT,
  thread_id TEXT,
  created_at INTEGER NOT NULL,
  UNIQUE(mindmap, node)
);
CREATE TABLE IF NOT EXISTS agent_jobs (
  id TEXT PRIMARY KEY,
  conversation_id TEXT NOT NULL REFERENCES agent_conversations(id) ON DELETE CASCADE,
  requested_by TEXT NOT NULL,
  request_id TEXT NOT NULL,
  prompt TEXT NOT NULL,
  snapshot TEXT NOT NULL,
  source_revision TEXT NOT NULL,
  status TEXT NOT NULL CHECK(status IN ('queued','running','completed','failed')),
  attempt_id TEXT,
  service_id TEXT,
  token_id TEXT,
  thread_id TEXT,
  turn_id TEXT,
  lease_expires_at INTEGER,
  deadline INTEGER,
  error TEXT,
  result_json TEXT,
  created_at INTEGER NOT NULL,
  finished_at INTEGER,
  UNIQUE(conversation_id, requested_by, request_id)
);
CREATE UNIQUE INDEX IF NOT EXISTS agent_one_active_turn
  ON agent_jobs(conversation_id) WHERE status IN ('queued','running');
CREATE INDEX IF NOT EXISTS agent_queue ON agent_jobs(status, created_at);
CREATE TABLE IF NOT EXISTS agent_messages (
  id TEXT PRIMARY KEY,
  conversation_id TEXT NOT NULL REFERENCES agent_conversations(id) ON DELETE CASCADE,
  job_id TEXT NOT NULL REFERENCES agent_jobs(id) ON DELETE CASCADE,
  role TEXT NOT NULL CHECK(role IN ('user','assistant')),
  body TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  UNIQUE(job_id, role)
);
CREATE INDEX IF NOT EXISTS agent_message_order ON agent_messages(conversation_id, created_at);
