-- Replace legacy unconditional triggers after additive columns exist.
DROP TRIGGER IF EXISTS ticket_document_created;
DROP TRIGGER IF EXISTS ticket_document_edited;
CREATE TRIGGER ticket_document_created AFTER INSERT ON tickets
WHEN COALESCE((SELECT scheduling FROM ticket_document_settings WHERE project=NEW.project),'automatic')='automatic' BEGIN
 INSERT OR IGNORE INTO ticket_document_pending(ticket) VALUES(NEW.id);
END;
CREATE TRIGGER ticket_document_edited AFTER UPDATE OF title,body,project,parent ON tickets
WHEN (OLD.title<>NEW.title OR OLD.body<>NEW.body OR OLD.project<>NEW.project OR OLD.parent IS NOT NEW.parent)
 AND COALESCE((SELECT scheduling FROM ticket_document_settings WHERE project=NEW.project),'automatic')='automatic' BEGIN
 INSERT OR IGNORE INTO ticket_document_pending(ticket) VALUES(NEW.id);
END;
-- A request made in one project must not authorize matching in another.
DROP TRIGGER IF EXISTS ticket_document_pending_moved;
CREATE TRIGGER ticket_document_pending_moved BEFORE UPDATE OF project ON tickets
WHEN OLD.project<>NEW.project BEGIN
 DELETE FROM ticket_document_pending WHERE ticket=OLD.id;
END;
