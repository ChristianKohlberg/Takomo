CREATE TABLE IF NOT EXISTS lane_organizer_conversations (
 project TEXT PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
 conversation TEXT NOT NULL UNIQUE REFERENCES agent_conversations(id) ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS lane_organizer_jobs (
 job TEXT PRIMARY KEY REFERENCES agent_jobs(id) ON DELETE CASCADE,
 proposal TEXT, accepted_at INTEGER, accepted_by TEXT
);
