-- Distinct from historical verification `lanes` (now checks).
CREATE TABLE IF NOT EXISTS work_lanes (
 id TEXT PRIMARY KEY, project TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
 data TEXT NOT NULL, updated_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS work_lanes_project ON work_lanes(project,updated_at);
CREATE TABLE IF NOT EXISTS work_lane_tickets (
 lane TEXT NOT NULL REFERENCES work_lanes(id) ON DELETE CASCADE,
 ticket TEXT NOT NULL REFERENCES tickets(id) ON DELETE CASCADE,
 PRIMARY KEY(lane,ticket)
);
CREATE TABLE IF NOT EXISTS work_handoffs (
 id TEXT PRIMARY KEY, project TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
 lane TEXT NOT NULL REFERENCES work_lanes(id) ON DELETE CASCADE,
 status TEXT NOT NULL, lease_until INTEGER, owner_token TEXT, owner_actor TEXT,
 data TEXT NOT NULL, created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS work_handoffs_queue ON work_handoffs(project,status,lease_until);
CREATE INDEX IF NOT EXISTS work_handoffs_lane ON work_handoffs(lane,created_at);
