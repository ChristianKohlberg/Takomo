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
