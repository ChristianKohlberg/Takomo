//! Per-project workflow (state machine) definition, validation, and the
//! built-in `factory-default` workflow. See spec/workflow-format.md.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

pub const CATEGORIES: [&str; 6] = [
    "todo",
    "in_progress",
    "blocked",
    "review",
    "done",
    "cancelled",
];

/// v1 server-side guards.
pub const GUARDS: [&str; 2] = ["no_open_children", "no_open_blockers"];

// deny_unknown_fields everywhere: a typo like "require" or "claimble" must be
// a 422, not a silently deleted approval gate.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowState {
    pub id: String,
    pub category: String,
    #[serde(default)]
    pub claimable: bool,
    #[serde(default)]
    pub terminal: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowTransition {
    pub from: String,
    pub to: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Workflow {
    pub name: String,
    pub initial: String,
    pub states: Vec<WorkflowState>,
    pub transitions: Vec<WorkflowTransition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guards: Option<serde_json::Value>,
}

/// A single `requires` entry, parsed.
#[derive(Debug, Clone, PartialEq)]
pub enum Requirement {
    Claim,
    Scope(String),
    Guard(String),
}

impl Requirement {
    pub fn parse(raw: &str) -> Result<Requirement, String> {
        if raw == "claim" {
            Ok(Requirement::Claim)
        } else if let Some(scope) = raw.strip_prefix("scope:") {
            if scope.is_empty() {
                Err(format!("empty scope in requirement '{raw}'"))
            } else {
                Ok(Requirement::Scope(scope.to_string()))
            }
        } else if let Some(guard) = raw.strip_prefix("guard:") {
            if GUARDS.contains(&guard) {
                Ok(Requirement::Guard(guard.to_string()))
            } else {
                Err(format!(
                    "unknown guard '{guard}' in requirement '{raw}'; v1 guards are: {}",
                    GUARDS.join(", ")
                ))
            }
        } else {
            Err(format!(
                "unknown requirement '{raw}'; must be 'claim', 'scope:<scope>', or 'guard:<id>'"
            ))
        }
    }
}

impl Workflow {
    pub fn state(&self, id: &str) -> Option<&WorkflowState> {
        self.states.iter().find(|s| s.id == id)
    }

    /// All transition edges leaving `from`.
    pub fn transitions_from(&self, from: &str) -> Vec<&WorkflowTransition> {
        self.transitions.iter().filter(|t| t.from == from).collect()
    }

    /// Validate structural integrity. `existing_states_in_use` are ticket
    /// states currently present in the project; the workflow must still define
    /// all of them (never strand a ticket). Returns a list of human/LLM-legible
    /// problems; empty means valid.
    pub fn validate(&self, existing_states_in_use: &[String]) -> Vec<String> {
        let mut problems = Vec::new();

        if self.name.trim().is_empty() {
            problems.push("workflow 'name' must be non-empty".to_string());
        }
        if self.states.is_empty() {
            problems.push("workflow must define at least one state".to_string());
            return problems;
        }

        let mut seen = HashSet::new();
        for s in &self.states {
            if !seen.insert(s.id.as_str()) {
                problems.push(format!("duplicate state id '{}'", s.id));
            }
            if !CATEGORIES.contains(&s.category.as_str()) {
                problems.push(format!(
                    "state '{}' has unknown category '{}'; must be one of: {}",
                    s.id,
                    s.category,
                    CATEGORIES.join(", ")
                ));
            }
            if s.terminal && s.claimable {
                problems.push(format!(
                    "state '{}' is both terminal and claimable; terminal states end the lifecycle and cannot enter the ready queue",
                    s.id
                ));
            }
            if s.claimable && matches!(s.category.as_str(), "done" | "cancelled") {
                problems.push(format!(
                    "state '{}' is claimable with category '{}'; entering a done/cancelled-category state auto-releases the claim, so a claimable one would loop through the ready queue forever",
                    s.id, s.category
                ));
            }
        }

        let ids: HashSet<&str> = self.states.iter().map(|s| s.id.as_str()).collect();

        if !ids.contains(self.initial.as_str()) {
            problems.push(format!(
                "initial state '{}' is not defined in 'states'",
                self.initial
            ));
        }

        let terminal_ids: HashSet<&str> = self
            .states
            .iter()
            .filter(|s| s.terminal)
            .map(|s| s.id.as_str())
            .collect();

        for t in &self.transitions {
            if terminal_ids.contains(t.from.as_str()) {
                problems.push(format!(
                    "transition {} -> {} leaves terminal state '{}'; terminal states end the lifecycle and have no outgoing transitions",
                    t.from, t.to, t.from
                ));
            }
            if !ids.contains(t.from.as_str()) {
                problems.push(format!(
                    "transition {} -> {} references unknown 'from' state '{}'",
                    t.from, t.to, t.from
                ));
            }
            if !ids.contains(t.to.as_str()) {
                problems.push(format!(
                    "transition {} -> {} references unknown 'to' state '{}'",
                    t.from, t.to, t.to
                ));
            }
            for r in &t.requires {
                if let Err(e) = Requirement::parse(r) {
                    problems.push(format!("transition {} -> {}: {}", t.from, t.to, e));
                }
            }
        }

        // Every non-terminal state must have a path to a terminal state.
        // Reverse-BFS from terminal states over the transition graph.
        let mut reaches_terminal: HashSet<&str> = self
            .states
            .iter()
            .filter(|s| s.terminal)
            .map(|s| s.id.as_str())
            .collect();
        if reaches_terminal.is_empty() {
            problems.push(
                "workflow has no terminal state; at least one state needs 'terminal: true'"
                    .to_string(),
            );
        } else {
            // predecessors map
            let mut preds: HashMap<&str, Vec<&str>> = HashMap::new();
            for t in &self.transitions {
                preds
                    .entry(t.to.as_str())
                    .or_default()
                    .push(t.from.as_str());
            }
            let mut queue: Vec<&str> = reaches_terminal.iter().copied().collect();
            while let Some(node) = queue.pop() {
                if let Some(ps) = preds.get(node) {
                    for p in ps {
                        if reaches_terminal.insert(p) {
                            queue.push(p);
                        }
                    }
                }
            }
            for s in &self.states {
                if !s.terminal && !reaches_terminal.contains(s.id.as_str()) {
                    problems.push(format!(
                        "state '{}' has no path to any terminal state; tickets entering it would be stranded",
                        s.id
                    ));
                }
            }
        }

        // Existing tickets must not be stranded in states the workflow no
        // longer defines.
        let stranded: Vec<&String> = existing_states_in_use
            .iter()
            .filter(|s| !ids.contains(s.as_str()))
            .collect();
        if !stranded.is_empty() {
            problems.push(format!(
                "existing tickets sit in states this workflow no longer defines: {}; migrate those tickets first",
                stranded
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }

        problems
    }
}

/// The built-in `factory-default` workflow from spec/workflow-format.md.
pub fn factory_default() -> Workflow {
    let json = serde_json::json!({
        "name": "factory-default",
        "initial": "brief",
        "states": [
            { "id": "brief",          "category": "todo" },
            { "id": "spec",           "category": "in_progress", "claimable": true },
            { "id": "needs-decision", "category": "blocked" },
            { "id": "ready",          "category": "todo",        "claimable": true },
            { "id": "implementing",   "category": "in_progress" },
            { "id": "review",         "category": "review" },
            { "id": "done",           "category": "done",        "terminal": true },
            { "id": "cancelled",      "category": "cancelled",   "terminal": true }
        ],
        "transitions": [
            { "from": "brief",          "to": "spec" },
            { "from": "brief",          "to": "cancelled" },
            { "from": "spec",           "to": "needs-decision" },
            { "from": "needs-decision", "to": "spec",           "requires": ["scope:human"] },
            { "from": "spec",           "to": "ready",          "requires": ["scope:human"] },
            { "from": "spec",           "to": "cancelled" },
            { "from": "ready",          "to": "implementing",   "requires": ["claim"] },
            { "from": "implementing",   "to": "needs-decision" },
            { "from": "implementing",   "to": "review",         "requires": ["claim"] },
            { "from": "implementing",   "to": "ready" },
            { "from": "needs-decision", "to": "ready",          "requires": ["scope:human"] },
            { "from": "needs-decision", "to": "implementing",   "requires": ["scope:human"] },
            { "from": "review",         "to": "implementing" },
            { "from": "review",         "to": "done",           "requires": ["scope:human", "guard:no_open_children"] },
            { "from": "review",         "to": "cancelled",      "requires": ["scope:human"] },
            { "from": "ready",          "to": "cancelled" }
        ],
        "guards": {
            "no_open_children": {
                "description": "every child ticket must be in a terminal state"
            }
        }
    });
    serde_json::from_value(json).expect("factory-default workflow is valid")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn factory_default_is_valid() {
        let wf = factory_default();
        assert!(wf.validate(&[]).is_empty());
    }

    #[test]
    fn rejects_unreachable_terminal() {
        let mut wf = factory_default();
        // Remove all transitions out of 'brief' so it cannot reach terminal.
        wf.transitions.retain(|t| t.from != "brief");
        let problems = wf.validate(&[]);
        assert!(problems.iter().any(|p| p.contains("brief")));
    }

    #[test]
    fn rejects_stranded_tickets() {
        let wf = factory_default();
        let problems = wf.validate(&["legacy-state".to_string()]);
        assert!(problems.iter().any(|p| p.contains("legacy-state")));
    }
}
