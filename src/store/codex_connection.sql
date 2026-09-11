CREATE TABLE IF NOT EXISTS codex_connections (
 id TEXT PRIMARY KEY,
 token_id TEXT NOT NULL REFERENCES tokens(id) ON DELETE CASCADE,
 service_id TEXT NOT NULL,
 projects TEXT NOT NULL,
 seen_at INTEGER NOT NULL,
 busy INTEGER NOT NULL DEFAULT 0,
 report TEXT NOT NULL DEFAULT '{"status":"unknown"}',
 command_id TEXT,
 action TEXT,
 requested_by TEXT,
 expires_at INTEGER,
 UNIQUE(token_id, service_id)
);
