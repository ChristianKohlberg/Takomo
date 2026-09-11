//! Committed, project-scoped invalidations. TEMP triggers cover the single writer
//! centrally, including cascades and old/new ownership on moves. Unknown tables
//! deliberately fall back to a full refresh; caches and credentials are explicit.
use rusqlite::Connection;
use std::collections::BTreeSet;

#[derive(Clone, Debug)]
pub struct Change {
    pub project: String,
    pub topic: String,
}

fn rule(table: &str) -> Option<(&'static str, &'static str)> {
    Some(match table {
        "codex_connections" | "idempotency" | "comment_idempotency" | "crdt_sessions" | "oauth_clients"
        | "oauth_codes" | "oauth_refresh" | "oauth_issued"
        | "search_nodes" | "search_chunks" | "ticket_document_pending"
        | "specification_history_heads" | "query_embedding_cache" | "query_cache_generation" => return None,
        t if t.starts_with("search_fts") => return None,
        "tokens" | "users" => ("''", ""),
        "user_projects" | "shares" => ("@.project", ""),
        "answer_grants" => ("@.project", "inbox"),
        "projects" | "workflow_library" => ("''", "projects"),
        // Readiness can depend on a blocker in another project.
        "tickets" | "deps" => ("''", "tickets"),
        "workflow_states" => ("''", "projects,tickets"),
        "questions" => ("@.project", "inbox"),
        "question_messages" => ("(SELECT project FROM questions WHERE id=@.question)", "inbox"),
        "mindmaps" => ("@.project", "document,trace,tickets,agent,history,search"),
        "mindmap_nodes" | "document_agent_settings" => ("(SELECT project FROM mindmaps WHERE id=@.mindmap)", "document,trace,tickets,agent"),
        "plan_trace" => ("@.project", "trace,document"),
        "specification_versions" | "specification_checkpoints" => ("(SELECT project FROM mindmaps WHERE id=@.mindmap)", "history"),
        "crdt_updates" => ("CASE @.object_kind WHEN 'mindmap' THEN (SELECT project FROM mindmaps WHERE id=@.object_id) WHEN 'document' THEN (SELECT project FROM documents WHERE id=@.object_id) WHEN 'check' THEN (SELECT project FROM checks WHERE id=@.object_id) ELSE NULL END", "document,trace,tickets,checks"),
        "checks" | "checklist_policies" | "test_specification_revisions" | "test_runs" | "environments" | "releases" => ("@.project", "checks,document"),
        "cases" | "check_globs" | "check_environments" | "test_definition_revisions" => ("(SELECT project FROM checks WHERE id=@.check_id)", "checks,document"),
        "case_verdicts" | "case_environments" => ("(SELECT c.project FROM checks c JOIN cases x ON x.check_id=c.id WHERE x.id=@.case_id)", "checks,document"),
        "test_run_cases" | "test_run_results" => ("(SELECT project FROM test_runs WHERE id=@.run_id)", "checks,document"),
        "release_paths" | "release_orphan_globs" => ("(SELECT project FROM releases WHERE id=@.release)", "checks"),
        "codebase_import_jobs" => ("@.project", "agent"),
        "agent_run_usage" => ("COALESCE((SELECT project FROM codebase_import_jobs WHERE id=@.job),(SELECT c.project FROM agent_jobs j JOIN agent_conversations c ON c.id=j.conversation_id WHERE j.id=@.job))", "agent"),
        "agent_conversations" | "lane_organizer_conversations" => ("@.project", "agent,tickets"),
        "agent_jobs" | "agent_messages" | "document_thread_profiles" => ("(SELECT project FROM agent_conversations WHERE id=@.conversation_id)", "agent,tickets"),
        "document_agent_jobs" | "document_workspace_jobs" | "document_thread_migrations" | "lane_organizer_jobs" | "bug_research_jobs" | "agent_steering" | "ticket_document_jobs" => ("(SELECT c.project FROM agent_conversations c JOIN agent_jobs j ON j.conversation_id=c.id WHERE j.id=@.job)", "agent,tickets"),
        "ticket_document_links" | "ticket_document_state" | "bug_triage" | "bug_research_requests" | "comments" => ("(SELECT project FROM tickets WHERE id=@.ticket)", "tickets"),
        "ticket_document_settings" | "work_lanes" | "work_handoffs" | "schedules" | "promotions" => ("@.project", "tickets,agent"),
        "work_lane_tickets" => ("(SELECT project FROM work_lanes WHERE id=@.lane)", "tickets"),
        "bug_research_config" => ("@.project", "projects,tickets,agent"),
        "project_writing_instructions" => ("@.project", "projects"),
        "events" => ("@.project", "history"),
        "tags" | "initiatives" | "initiative_entries" | "documents" => ("@.project", ""),
        "embedding_settings" => ("''", "search"),
        "search_dirty_maps" | "embedding_jobs" | "search_failures" | "embedding_sync_history" => ("(SELECT project FROM mindmaps WHERE id=@.map_id)", "search"),
        _ => ("''", ""),
    })
}

// These FK children may run AFTER their parent was removed. Its BEFORE
// trigger already captured ownership and a superset of these topics.
fn cascade_owner(table: &str) -> bool {
    matches!(
        table,
        "question_messages"
            | "mindmap_nodes"
            | "document_agent_settings"
            | "specification_versions"
            | "specification_checkpoints"
            | "cases"
            | "check_globs"
            | "check_environments"
            | "test_definition_revisions"
            | "case_verdicts"
            | "case_environments"
            | "test_run_cases"
            | "test_run_results"
            | "release_paths"
            | "release_orphan_globs"
            | "agent_run_usage"
            | "codebase_import_jobs"
            | "agent_jobs"
            | "agent_messages"
            | "document_thread_profiles"
            | "document_agent_jobs"
            | "document_workspace_jobs"
            | "document_thread_migrations"
            | "lane_organizer_jobs"
            | "bug_research_jobs"
            | "agent_steering"
            | "ticket_document_jobs"
            | "ticket_document_links"
            | "ticket_document_state"
            | "bug_triage"
            | "bug_research_requests"
            | "comments"
            | "work_lane_tickets"
            | "search_dirty_maps"
            | "embedding_jobs"
            | "search_failures"
            | "embedding_sync_history"
    )
}

fn quoted(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

pub fn install(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch("CREATE TEMP TABLE live_changes(project TEXT NOT NULL, topic TEXT NOT NULL, PRIMARY KEY(project,topic)) WITHOUT ROWID;")?;
    let tables = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'")?
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for table in tables {
        let Some((owner, topics)) = rule(&table) else {
            continue;
        };
        let columns = conn
            .prepare(&format!("PRAGMA table_info({})", quoted(&table)))?
            .query_map([], |r| r.get::<_, String>(1))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let changed = columns
            .iter()
            .filter(|c| {
                !(matches!(table.as_str(), "tokens" | "shares" | "answer_grants")
                    && c.as_str() == "last_used_at"
                    || table == "agent_jobs" && c.as_str() == "lease_expires_at"
                    || table == "embedding_jobs" && c.as_str() == "lease_until")
            })
            .map(|c| format!("OLD.{0} IS NOT NEW.{0}", quoted(c)))
            .collect::<Vec<_>>()
            .join(" OR ");
        for action in ["INSERT", "UPDATE", "DELETE"] {
            let rows: &[&str] = match action {
                "INSERT" => &["NEW"],
                "DELETE" => &["OLD"],
                _ => &["OLD", "NEW"],
            };
            let mut body = String::new();
            for row in rows {
                let project = owner.replace('@', row);
                for topic in topics.split(',') {
                    let guard = if action == "DELETE" && cascade_owner(&table) {
                        format!(" AND ({project}) IS NOT NULL")
                    } else {
                        String::new()
                    };
                    body.push_str(&format!("INSERT INTO live_changes SELECT COALESCE({project},''), '{topic}' WHERE NOT EXISTS (SELECT 1 FROM live_changes WHERE project=COALESCE({project},'') AND topic='{topic}'){guard};"));
                }
                // Archive/delete and workflow settings can invalidate the whole view.
                if table == "projects" {
                    body.push_str(&format!(
                        "INSERT INTO live_changes SELECT {row}.id,'' WHERE NOT EXISTS (SELECT 1 FROM live_changes WHERE project={row}.id AND topic='');"
                    ));
                }
            }
            let when = if action == "UPDATE" {
                format!(" WHEN {changed}")
            } else {
                String::new()
            };
            let timing = if action == "DELETE" {
                "BEFORE"
            } else {
                "AFTER"
            };
            conn.execute_batch(&format!(
                "CREATE TEMP TRIGGER {} {timing} {action} ON main.{}{when} BEGIN {body} END;",
                quoted(&format!("live_{table}_{action}")),
                quoted(&table)
            ))?;
        }
    }
    Ok(())
}

pub fn drain(conn: &Connection) -> rusqlite::Result<Vec<Change>> {
    let changes = conn
        .prepare("SELECT project,topic FROM temp.live_changes")?
        .query_map([], |r| {
            Ok(Change {
                project: r.get(0)?,
                topic: r.get(1)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    conn.execute("DELETE FROM temp.live_changes", [])?;
    Ok(changes)
}

pub fn topics(changes: &[Change], project: &str, into: &mut BTreeSet<String>) {
    for change in changes {
        if change.project.is_empty() || change.project == project {
            into.insert(change.topic.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{error::ApiError, store::Store};

    #[test]
    fn journal_tracks_committed_ownership_and_ignores_usage() {
        let store = Store::open(":memory:").unwrap();
        store.create_project("aa", "A", None, "test").unwrap();
        store.create_project("bb", "B", None, "test").unwrap();
        let mut rx = store.live_changes.subscribe();
        store.with_tx(|tx| {
            tx.execute("INSERT INTO mindmaps(id,project,title,created_by,created_at,updated_at) VALUES('m','aa','Map','test',1,1)", [])?;
            Ok(())
        }).unwrap();
        let batch = rx.try_recv().unwrap();
        assert!(batch
            .iter()
            .any(|c| c.project == "aa" && c.topic == "document"));
        assert!(batch.iter().all(|c| c.project == "aa"));
        store.with_tx(|tx| {
            tx.execute("INSERT OR IGNORE INTO mindmaps(id,project,title,created_by,created_at,updated_at) VALUES('m','aa','Ignored','test',1,1)", [])?;
            Ok(())
        }).unwrap();
        assert!(rx.try_recv().is_err(), "ignored INSERT must not notify");
        store.with_tx(|tx| {
            tx.execute("INSERT INTO mindmaps(id,project,title,created_by,created_at,updated_at) VALUES('second','bb','Existing','test',1,1)", [])?;
            Ok(())
        }).unwrap();
        rx.try_recv().unwrap();
        store
            .with_tx(|tx| {
                tx.execute(
                    "UPDATE OR IGNORE mindmaps SET id='second', project='bb' WHERE id='m'",
                    [],
                )?;
                Ok(())
            })
            .unwrap();
        assert!(rx.try_recv().is_err(), "ignored UPDATE must not notify");
        store
            .with_tx(|tx| {
                // Outer conflict policies override a trigger's OR IGNORE policy.
                // Deduplication must avoid attempting a duplicate insert at all.
                tx.execute(
                    "UPDATE OR ABORT mindmaps SET title='first' WHERE id='m'",
                    [],
                )?;
                tx.execute(
                    "UPDATE OR ABORT mindmaps SET title='second' WHERE id='m'",
                    [],
                )?;
                Ok(())
            })
            .unwrap();
        let batch = rx.try_recv().unwrap();
        assert_eq!(
            batch
                .iter()
                .filter(|c| c.project == "aa" && c.topic == "document")
                .count(),
            1
        );
        let failed: crate::error::ApiResult<()> = store.with_tx(|tx| {
            tx.execute("UPDATE mindmaps SET title='rolled back' WHERE id='m'", [])?;
            Err(ApiError::internal("fixture rollback"))
        });
        assert!(failed.is_err());
        store
            .with_tx(|tx| {
                tx.execute("UPDATE mindmaps SET title=title WHERE id='m'", [])?;
                Ok(())
            })
            .unwrap();
        assert!(rx.try_recv().is_err());
        store
            .with_tx(|tx| {
                tx.execute("UPDATE mindmaps SET project='bb' WHERE id='m'", [])?;
                Ok(())
            })
            .unwrap();
        let batch = rx.try_recv().unwrap();
        for project in ["aa", "bb"] {
            assert!(batch
                .iter()
                .any(|c| c.project == project && c.topic == "document"));
        }
        store.with_tx(|tx| {
            tx.execute("INSERT INTO tokens(id,hash,actor,scopes,projects,created_at) VALUES('tok','hash','fixture','[]','[]',1)", [])?;
            Ok(())
        }).unwrap();
        rx.try_recv().unwrap();
        store
            .with_tx(|tx| {
                tx.execute("UPDATE tokens SET last_used_at=20 WHERE id='tok'", [])?;
                Ok(())
            })
            .unwrap();
        assert!(rx.try_recv().is_err());
        store
            .with_tx(|tx| {
                tx.execute("UPDATE tokens SET revoked_at=21 WHERE id='tok'", [])?;
                Ok(())
            })
            .unwrap();
        assert!(rx
            .try_recv()
            .unwrap()
            .iter()
            .any(|c| c.project.is_empty() && c.topic.is_empty()));
    }
    #[test]
    fn cascades_keep_parent_topics_without_unrelated_project_refreshes() {
        let store = Store::open(":memory:").unwrap();
        store.create_project("aa", "A", None, "test").unwrap();
        store.create_project("bb", "B", None, "test").unwrap();
        store.with_tx(|tx| { tx.execute_batch("
            INSERT INTO mindmaps(id,project,title,created_by,created_at,updated_at) VALUES('m','aa','Map','test',1,1);
            INSERT INTO document_agent_settings(mindmap,pinned_section_ids) VALUES('m','[]');
            INSERT INTO checks(id,project,title,created_by,created_at,updated_at) VALUES('ck','aa','Check','test',1,1);
            INSERT INTO cases(id,check_id,key,created_at,updated_at) VALUES('ca','ck','default',1,1);
            INSERT INTO case_verdicts(id,case_id,actor_kind,actor,verdict,at) VALUES('cv','ca','agent','test','pass',1);
            INSERT INTO tickets(id,project,title,state,created_by,created_at,updated_at) VALUES('aa-1','aa','Ticket','open','test',1,1);
            INSERT INTO agent_conversations(id,ticket,node,project,created_at) VALUES('co','aa-1','ticket','aa',1);
            INSERT INTO agent_jobs(id,conversation_id,requested_by,request_id,prompt,snapshot,source_revision,status,created_at) VALUES('j','co','test','req','x','{}','rev','queued',1);
            INSERT INTO document_agent_jobs(job,action,section_ids,whole_document) VALUES('j','discuss','[]',1);
        ")?;Ok(()) }).unwrap();
        let mut rx = store.live_changes.subscribe();
        for (table, id, topic) in [
            ("mindmaps", "m", "document"),
            ("checks", "ck", "checks"),
            ("tickets", "aa-1", "agent"),
        ] {
            store
                .with_tx(|tx| {
                    tx.execute(&format!("DELETE FROM {table} WHERE id=?1"), [id])?;
                    Ok(())
                })
                .unwrap();
            let batch = rx.try_recv().unwrap();
            assert!(
                batch.iter().any(|c| c.project == "aa" && c.topic == topic),
                "{table}: {batch:?}"
            );
            assert!(
                batch
                    .iter()
                    .all(|c| c.project == "aa" || c.project.is_empty() && c.topic == "tickets"),
                "{table}: {batch:?}"
            );
        }
    }
    #[test]
    fn query_cache_bookkeeping_stays_silent_but_settings_remain_live() {
        let store = Store::open(":memory:").unwrap();
        store.create_project("tp", "Test", None, "test").unwrap();
        let legacy = store.changes.subscribe();
        let mut live = store.live_changes.subscribe();
        let generation = store.query_cache_generation().unwrap();
        store
            .cache_query_vector("fixture", generation, &[1.0, 0.0], 10000, 1, 1)
            .unwrap();
        assert!(store
            .cached_query_vector("fixture", generation, 2, 2)
            .unwrap()
            .is_some());
        // Eviction and expiry, not only insertion and cache-hit usage touches.
        store
            .cache_query_vector("replacement", generation, &[0.0, 1.0], 20000, 3, 1)
            .unwrap();
        store
            .cache_query_vector("expired", generation, &[0.0, 1.0], 30000, 20001, 1)
            .unwrap();
        assert!(!legacy.has_changed().unwrap());
        assert!(live.try_recv().is_err());
        // Explicit inventory exclusions also apply to cache writes via with_tx.
        store
            .with_tx(|tx| {
                tx.execute(
                    "UPDATE query_cache_generation SET generation=generation+1 WHERE id=1",
                    [],
                )?;
                Ok(())
            })
            .unwrap();
        assert!(live.try_recv().is_err());
        let (config, _) = store.embedding_config().unwrap();
        store
            .save_embedding_config(config, Some("fixture-key".into()))
            .unwrap();
        let batch = live.try_recv().expect("real settings changes still notify");
        assert!(batch.iter().any(|c| c.topic == "search"));
        assert!(batch.iter().all(|c| !c.topic.is_empty()));
        store.with_tx(|tx| {tx.execute("INSERT INTO bug_research_config(project,repository,revision,enabled) VALUES('tp','fixture','main',1)",[])?;Ok(())}).unwrap();
        let batch = live.try_recv().unwrap();
        assert!(batch
            .iter()
            .any(|c| c.project == "tp" && c.topic == "projects"));
        assert!(batch.iter().all(|c| c.project == "tp"));
    }
    #[test]
    fn silent_transactions_drain_even_conservative_future_table_notifications() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fixture.db");
        rusqlite::Connection::open(&path)
            .unwrap()
            .execute(
                "CREATE TABLE future_cache(id INTEGER PRIMARY KEY,value TEXT)",
                [],
            )
            .unwrap();
        let store = Store::open(&path).unwrap();
        let mut live = store.live_changes.subscribe();
        store
            .cache_transaction(|tx| {
                tx.execute("INSERT INTO future_cache VALUES(1,'cached')", [])?;
                Ok(())
            })
            .unwrap();
        assert!(live.try_recv().is_err());
        store.with_tx(|_| Ok(())).unwrap();
        assert!(
            live.try_recv().is_err(),
            "suppressed journal entries must not leak to a later transaction"
        );
        store
            .with_tx(|tx| {
                tx.execute(
                    "UPDATE future_cache SET value='real unknown mutation' WHERE id=1",
                    [],
                )?;
                Ok(())
            })
            .unwrap();
        assert!(
            live.try_recv()
                .unwrap()
                .iter()
                .any(|c| c.project.is_empty() && c.topic.is_empty()),
            "ordinary unknown content still gets conservative recovery"
        );
    }
}
