CREATE TABLE IF NOT EXISTS embedding_settings (
 id INTEGER PRIMARY KEY CHECK(id=1), config TEXT NOT NULL, api_key TEXT NOT NULL DEFAULT '', fingerprint TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS search_dirty_maps (
 map_id TEXT PRIMARY KEY REFERENCES mindmaps(id) ON DELETE CASCADE, changed_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS search_nodes (
 map_id TEXT NOT NULL REFERENCES mindmaps(id) ON DELETE CASCADE, node_id TEXT NOT NULL,
 content_hash TEXT NOT NULL, PRIMARY KEY(map_id,node_id)
);
CREATE TABLE IF NOT EXISTS search_chunks (
 id INTEGER PRIMARY KEY, map_id TEXT NOT NULL, node_id TEXT NOT NULL, ordinal INTEGER NOT NULL,
 title TEXT NOT NULL, heading_path TEXT NOT NULL, passage TEXT NOT NULL, content_hash TEXT NOT NULL,
 fingerprint TEXT NOT NULL DEFAULT '', vector TEXT,
 FOREIGN KEY(map_id,node_id) REFERENCES search_nodes(map_id,node_id) ON DELETE CASCADE,
 UNIQUE(map_id,node_id,ordinal)
);
CREATE VIRTUAL TABLE IF NOT EXISTS search_fts USING fts5(title, heading_path, passage, content='search_chunks', content_rowid='id', tokenize='unicode61');
CREATE TRIGGER IF NOT EXISTS search_chunks_insert AFTER INSERT ON search_chunks BEGIN
 INSERT INTO search_fts(rowid,title,heading_path,passage) VALUES(new.id,new.title,new.heading_path,new.passage);
END;
CREATE TRIGGER IF NOT EXISTS search_chunks_delete AFTER DELETE ON search_chunks BEGIN
 INSERT INTO search_fts(search_fts,rowid,title,heading_path,passage) VALUES('delete',old.id,old.title,old.heading_path,old.passage);
END;
CREATE TABLE IF NOT EXISTS embedding_jobs (
 map_id TEXT NOT NULL, node_id TEXT NOT NULL, content_hash TEXT NOT NULL, fingerprint TEXT NOT NULL,
 first_changed INTEGER NOT NULL, due_at INTEGER NOT NULL, lease_until INTEGER NOT NULL DEFAULT 0,
 attempts INTEGER NOT NULL DEFAULT 0, last_error TEXT,
 PRIMARY KEY(map_id,node_id), FOREIGN KEY(map_id,node_id) REFERENCES search_nodes(map_id,node_id) ON DELETE CASCADE
);
CREATE TRIGGER IF NOT EXISTS search_crdt_dirty AFTER INSERT ON crdt_updates WHEN new.object_kind='mindmap' BEGIN
 INSERT INTO search_dirty_maps(map_id,changed_at) VALUES(new.object_id,new.created_at)
 ON CONFLICT(map_id) DO UPDATE SET changed_at=excluded.changed_at;
END;
CREATE TABLE IF NOT EXISTS search_failures (
 map_id TEXT PRIMARY KEY REFERENCES mindmaps(id) ON DELETE CASCADE, failed_at INTEGER NOT NULL, message TEXT NOT NULL
);
-- Historical successful completion, never inferred from polling or source timestamps.
CREATE TABLE IF NOT EXISTS embedding_sync_history (
 map_id TEXT NOT NULL REFERENCES mindmaps(id) ON DELETE CASCADE,
 fingerprint TEXT NOT NULL, last_synced_at INTEGER NOT NULL,
 PRIMARY KEY(map_id,fingerprint)
);
