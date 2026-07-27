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

/// v1 server-side guards that take no parameter.
pub const GUARDS: [&str; 2] = ["no_open_children", "no_open_blockers"];

/// Prefix of the parameterized guard family `has_link:<key>`: the ticket must
/// carry a non-empty `links.<key>`. `has_link:commit` is the intended use — it
/// turns "done" from a claim into something a later reader can verify, because
/// the commit is checkable long after everyone involved has forgotten. The key
/// is free-form so a project can demand whatever its own proof is (`pr`,
/// `env`, `run`).
pub const GUARD_HAS_LINK: &str = "has_link:";

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
            } else if let Some(key) = guard.strip_prefix(GUARD_HAS_LINK) {
                if key.trim().is_empty() {
                    Err(format!(
                        "guard '{guard}' in requirement '{raw}' names no link key; write the key it must prove, e.g. 'guard:{GUARD_HAS_LINK}commit'"
                    ))
                } else {
                    Ok(Requirement::Guard(guard.to_string()))
                }
            } else {
                Err(format!(
                    "unknown guard '{guard}' in requirement '{raw}'; v1 guards are: {}, or '{GUARD_HAS_LINK}<key>' (e.g. '{GUARD_HAS_LINK}commit')",
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

/// The canonical text of the built-in workflow. `workflows/` is the one place
/// a shipped workflow is defined; embedding it here rather than re-typing it in
/// Rust is what keeps the file and the server from drifting apart. It is baked
/// in at compile time (like `src/board.html`), so a deployed binary never reads
/// this path at runtime.
pub const FACTORY_DEFAULT_YAML: &str = include_str!("../workflows/factory-default.yaml");

/// The built-in `factory-default` workflow. Format: spec/workflow-format.md;
/// definition: `workflows/factory-default.yaml`.
///
/// The parse cannot fail on a shipped file without failing for every caller at
/// once, so it panics rather than returning a Result — and
/// `factory_default_parses` below turns that into a test failure instead of a
/// startup failure.
pub fn factory_default() -> Workflow {
    serde_norway::from_str(FACTORY_DEFAULT_YAML)
        .expect("workflows/factory-default.yaml is a valid workflow")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shipped YAML files, by path relative to the crate root. `workflows/`
    /// is meant to be the only place a shipped workflow is defined, so every
    /// file in it must be a workflow the server would accept.
    const SHIPPED: [&str; 2] = ["workflows/factory-default.yaml", "workflows/simple.yaml"];

    fn read_shipped(rel: &str) -> String {
        std::fs::read_to_string(format!("{}/{rel}", env!("CARGO_MANIFEST_DIR")))
            .unwrap_or_else(|e| panic!("{rel}: {e}"))
    }

    #[test]
    fn factory_default_is_valid() {
        let wf = factory_default();
        assert!(wf.validate(&[]).is_empty());
    }

    /// `factory_default()` panics on a malformed file, which would take down
    /// every project creation at once. Fail here instead, at build time.
    #[test]
    fn factory_default_parses() {
        let wf: Workflow = serde_norway::from_str(FACTORY_DEFAULT_YAML)
            .expect("workflows/factory-default.yaml must parse");
        assert_eq!(wf.name, "factory-default");
        assert_eq!(wf.initial, "brief");
    }

    /// The embedded copy is `include_str!`'d at compile time, so a stale build
    /// could in principle disagree with the file on disk. Reading the file back
    /// at test time is what proves the two are the same text.
    #[test]
    fn embedded_factory_default_matches_the_file() {
        assert_eq!(
            FACTORY_DEFAULT_YAML,
            read_shipped("workflows/factory-default.yaml"),
            "src/workflow.rs embeds a different factory-default than workflows/ ships"
        );
    }

    /// Every shipped workflow must be one the server would accept on
    /// `PUT /v1/projects/{p}/workflow` — a file in `workflows/` that the server
    /// would 422 is worse than no file at all, because it reads as blessed.
    #[test]
    fn every_shipped_workflow_is_valid() {
        for rel in SHIPPED {
            let wf: Workflow = serde_norway::from_str(&read_shipped(rel))
                .unwrap_or_else(|e| panic!("{rel} does not parse: {e}"));
            let problems = wf.validate(&[]);
            assert!(problems.is_empty(), "{rel} is invalid: {problems:?}");
        }
    }

    /// Compare by parsed shape, not by text: the copies below are legitimately
    /// formatted differently (comments, JSON vs YAML), and only a difference in
    /// what the state machine *means* is a bug.
    fn shape(wf: &Workflow) -> serde_json::Value {
        serde_json::to_value(wf).expect("a workflow serializes")
    }

    /// spec/workflow-format.md prints factory-default inline so the format doc
    /// is readable on its own. That copy is prose, not the definition — this
    /// pins it to the file so the documented default cannot quietly become a
    /// different state machine from the shipped one.
    #[test]
    fn spec_doc_factory_default_matches_the_file() {
        let doc = read_shipped("spec/workflow-format.md");
        let block = doc
            .split("```yaml")
            .nth(1)
            .and_then(|rest| rest.split("```").next())
            .expect("spec/workflow-format.md has a ```yaml block");
        let documented: Workflow =
            serde_norway::from_str(block).expect("the doc's yaml block is a workflow");
        assert_eq!(
            shape(&documented),
            shape(&factory_default()),
            "spec/workflow-format.md documents a different factory-default than workflows/ ships"
        );
    }

    /// clients/cli/takomo embeds `simple` as JSON so `takomo init` still works
    /// when the CLI is symlinked away from the repo and cannot reach
    /// workflows/. The fallback is legitimate; silently applying a *different*
    /// state machine depending on how the CLI was installed is not.
    #[test]
    fn cli_embedded_simple_matches_the_file() {
        let cli = read_shipped("clients/cli/takomo");
        let body = cli
            .split("simple_workflow_json() {")
            .nth(1)
            .and_then(|rest| rest.split("cat <<'EOF'").nth(1))
            .and_then(|rest| rest.split("\nEOF").next())
            .expect("clients/cli/takomo defines simple_workflow_json with a heredoc");
        let embedded: Workflow =
            serde_json::from_str(body).expect("the CLI's embedded workflow is a workflow");
        let shipped: Workflow = serde_norway::from_str(&read_shipped("workflows/simple.yaml"))
            .expect("workflows/simple.yaml is a workflow");
        assert_eq!(
            shape(&embedded),
            shape(&shipped),
            "clients/cli/takomo embeds a different `simple` than workflows/simple.yaml"
        );
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
    fn parses_parameterized_has_link_guard() {
        assert_eq!(
            Requirement::parse("guard:has_link:commit"),
            Ok(Requirement::Guard("has_link:commit".to_string()))
        );
        // Any key, not just commit — a project decides what its proof is.
        assert_eq!(
            Requirement::parse("guard:has_link:pr"),
            Ok(Requirement::Guard("has_link:pr".to_string()))
        );
    }

    #[test]
    fn rejects_has_link_without_a_key() {
        for raw in ["guard:has_link:", "guard:has_link:   "] {
            let err = Requirement::parse(raw).unwrap_err();
            assert!(err.contains("names no link key"), "{raw}: {err}");
        }
    }

    #[test]
    fn unknown_guard_error_mentions_the_has_link_family() {
        let err = Requirement::parse("guard:has_commit").unwrap_err();
        assert!(err.contains("has_link:"), "{err}");
    }

    #[test]
    fn workflow_with_has_link_guard_validates() {
        let mut wf = factory_default();
        for t in wf.transitions.iter_mut() {
            if t.from == "review" && t.to == "done" {
                t.requires.push("guard:has_link:commit".to_string());
            }
        }
        assert!(wf.validate(&[]).is_empty(), "{:?}", wf.validate(&[]));
    }

    #[test]
    fn rejects_stranded_tickets() {
        let wf = factory_default();
        let problems = wf.validate(&["legacy-state".to_string()]);
        assert!(problems.iter().any(|p| p.contains("legacy-state")));
    }
}
