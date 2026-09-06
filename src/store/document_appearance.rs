//! A globally available typography preset with project-local numeric overrides.
use super::helpers::{emit_event, ensure_project_writable};
use super::{Project, Store};
use crate::error::{ApiError, ApiResult};
use rusqlite::params;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DocumentTemplate {
    #[default]
    Balanced,
    Strong,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DocumentAppearanceOverrides {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub h1_size: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub h2_size: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub h3_size: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body_size: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heading_weight: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_height: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heading_spacing: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DocumentAppearance {
    pub template: DocumentTemplate,
    pub overrides: DocumentAppearanceOverrides,
}

impl DocumentAppearance {
    fn validate(&self) -> ApiResult<()> {
        let o = &self.overrides;
        for (name, value, min, max) in [
            ("h1_size", o.h1_size, 12.0, 64.0),
            ("h2_size", o.h2_size, 12.0, 64.0),
            ("h3_size", o.h3_size, 12.0, 64.0),
            ("body_size", o.body_size, 12.0, 24.0),
            ("heading_weight", o.heading_weight, 400.0, 800.0),
            ("line_height", o.line_height, 1.0, 2.5),
            ("heading_spacing", o.heading_spacing, 0.0, 48.0),
        ] {
            if value.is_some_and(|v| {
                !v.is_finite()
                    || v < min
                    || v > max
                    || (name == "heading_weight" && v % 100.0 != 0.0)
            }) {
                return Err(ApiError::validation("project.document_appearance", format!(
                    "{name} must be a finite number between {min} and {max}; heading_weight must use increments of 100."
                )));
            }
        }
        Ok(())
    }
}

impl Store {
    pub fn set_document_appearance(
        &self,
        project: &str,
        appearance: DocumentAppearance,
        actor: &str,
    ) -> ApiResult<Project> {
        appearance.validate()?;
        let encoded = serde_json::to_string(&appearance)
            .map_err(|e| ApiError::internal(format!("Cannot encode document appearance: {e}")))?;
        self.with_tx(|tx| {
            ensure_project_writable(tx, project)?;
            let changed = tx.execute(
                "UPDATE projects SET document_appearance_json = ?2 WHERE id = ?1",
                params![project, encoded],
            )?;
            if changed == 0 {
                return Err(ApiError::not_found("project", project));
            }
            emit_event(
                tx,
                None,
                Some(project),
                actor,
                "document_appearance_changed",
                serde_json::json!({"document_appearance": appearance}),
                crate::ids::now_ms(),
            )?;
            Ok(())
        })?;
        self.get_project(project)?
            .ok_or_else(|| ApiError::not_found("project", project))
    }
}
