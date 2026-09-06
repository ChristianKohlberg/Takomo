//! Project-owned writing instructions, with no built-in content or outline.
use super::helpers::{emit_event, ensure_project_writable, get_workflow};
use super::Store;
use crate::error::{ApiError, ApiResult};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WritingInstruction {
    pub id: String,
    pub name: String,
    pub instruction: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WritingInstructions {
    pub templates: Vec<WritingInstruction>,
    pub default_id: Option<String>,
}

impl WritingInstructions {
    pub fn selected(&self) -> Option<&WritingInstruction> {
        self.default_id
            .as_ref()
            .and_then(|id| self.templates.iter().find(|t| &t.id == id))
    }

    fn normalize(mut self) -> ApiResult<Self> {
        let invalid = |message: &str| ApiError::validation("project.writing_instructions", message);
        if self.templates.len() > 20 {
            return Err(invalid(
                "A project can have at most 20 writing instructions.",
            ));
        }
        let mut ids = std::collections::HashSet::new();
        for template in &mut self.templates {
            if template.id.is_empty()
                || template.id.len() > 80
                || !template
                    .id
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
                || !ids.insert(template.id.clone())
            {
                return Err(invalid("Instruction ids must be unique and contain 1–80 ASCII letters, digits, underscores or hyphens."));
            }
            template.name = template.name.trim().to_string();
            template.instruction = template.instruction.trim().to_string();
            if template.name.is_empty() || template.name.chars().count() > 80 {
                return Err(invalid("An instruction name must contain 1–80 characters."));
            }
            if template.instruction.is_empty() || template.instruction.chars().count() > 4000 {
                return Err(invalid("An instruction must contain 1–4000 characters."));
            }
        }
        if self.default_id.is_some() && self.selected().is_none() {
            return Err(invalid(
                "default_id must identify an instruction in templates, or be null.",
            ));
        }
        Ok(self)
    }
}

impl Store {
    pub fn writing_instructions(&self, project: &str) -> ApiResult<WritingInstructions> {
        self.with_conn(|conn| {
            get_workflow(conn, project)?;
            let raw: Option<String> = conn
                .query_row(
                    "SELECT settings_json FROM project_writing_instructions WHERE project = ?1",
                    [project],
                    |row| row.get(0),
                )
                .optional()?;
            match raw {
                Some(raw) => Ok(serde_json::from_str(&raw)
                    .map_err(|_| ApiError::internal("Invalid stored writing instructions"))?),
                None => Ok(WritingInstructions::default()),
            }
        })
    }

    pub fn default_writing_instruction(
        &self,
        project: &str,
    ) -> ApiResult<Option<WritingInstruction>> {
        Ok(self.writing_instructions(project)?.selected().cloned())
    }

    /// Advisory project context: an explicit request always takes precedence.
    pub fn writing_prompt(&self, project: &str, request: &str) -> ApiResult<String> {
        Ok(match self.default_writing_instruction(project)? {
            Some(template) => format!("Project writing guidance (advisory; follow the user's explicit request when it conflicts):\n{}\n\nUser request:\n{}", template.instruction, request),
            None => request.to_string(),
        })
    }

    pub fn put_writing_instructions(
        &self,
        project: &str,
        settings: WritingInstructions,
        actor: &str,
    ) -> ApiResult<WritingInstructions> {
        let settings = settings.normalize()?;
        self.with_tx(|tx| {
            get_workflow(tx, project)?;
            ensure_project_writable(tx, project)?;
            tx.execute("INSERT INTO project_writing_instructions (project, settings_json) VALUES (?1, ?2) ON CONFLICT(project) DO UPDATE SET settings_json = excluded.settings_json", params![project, serde_json::to_string(&settings).expect("writing instructions serialize")])?;
            emit_event(tx, None, Some(project), actor, "writing_instructions_changed", serde_json::json!({"count": settings.templates.len(), "default_id": settings.default_id}), crate::ids::now_ms())?;
            Ok(())
        })?;
        Ok(settings)
    }
}
