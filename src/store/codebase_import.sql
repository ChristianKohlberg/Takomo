CREATE TABLE IF NOT EXISTS agent_run_usage (
 job TEXT PRIMARY KEY,
 telemetry TEXT NOT NULL,
 updated_at INTEGER NOT NULL,
 started_at INTEGER,
 finished_at INTEGER
);
CREATE TABLE IF NOT EXISTS github_connections (
 installation INTEGER PRIMARY KEY,
 account TEXT NOT NULL,
 management_url TEXT NOT NULL DEFAULT '',
 updated_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS project_repositories (
 project TEXT PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
 installation INTEGER NOT NULL REFERENCES github_connections(installation) ON DELETE CASCADE,
 repository INTEGER NOT NULL,
 full_name TEXT NOT NULL,
 scope TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS codebase_import_jobs (
 id TEXT PRIMARY KEY,
 project TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
 mindmap TEXT NOT NULL,
 actor TEXT NOT NULL,
 request_id TEXT NOT NULL,
 status TEXT NOT NULL CHECK(status IN ('queued','running','completed','failed')),
 source TEXT NOT NULL,
 created_at INTEGER NOT NULL,
 lease_expires_at INTEGER,
 deadline INTEGER,
 service_id TEXT,
 attempt_id TEXT,
 result TEXT,
 error TEXT,
 UNIQUE(project, request_id)
);
CREATE UNIQUE INDEX IF NOT EXISTS one_active_codebase_import ON codebase_import_jobs(project) WHERE status IN ('queued','running');

CREATE TRIGGER IF NOT EXISTS extraction_usage_cleanup BEFORE DELETE ON codebase_import_jobs BEGIN DELETE FROM agent_run_usage WHERE job=OLD.id; END;
CREATE TRIGGER IF NOT EXISTS agent_usage_cleanup BEFORE DELETE ON agent_jobs BEGIN DELETE FROM agent_run_usage WHERE job=OLD.id; END;
CREATE TRIGGER IF NOT EXISTS extraction_usage_finished AFTER UPDATE OF status ON codebase_import_jobs WHEN NEW.status IN ('completed','failed') AND OLD.status != NEW.status BEGIN UPDATE agent_run_usage SET finished_at=CAST(unixepoch('subsec')*1000 AS INTEGER) WHERE job=NEW.id; END;
CREATE TRIGGER IF NOT EXISTS agent_usage_started AFTER UPDATE OF status ON agent_jobs WHEN NEW.status='running' AND OLD.status='queued' BEGIN INSERT INTO agent_run_usage(job,telemetry,updated_at,started_at) VALUES(NEW.id,'{}',CAST(unixepoch('subsec')*1000 AS INTEGER),CAST(unixepoch('subsec')*1000 AS INTEGER)) ON CONFLICT(job) DO UPDATE SET started_at=excluded.started_at; END;
