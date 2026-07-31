//! QuantForge-native Retester task graph (SQX TaskRetest / ProjectRetester stand-in).
//!
//! SQX project/task XML lives inside proprietary plugin JARs without a public schema.
//! This module defines a portable JSON task graph that chains Retester-relevant steps:
//! Challenge → Walk-Forward matrix → M1 Judge → EA export.
//!
//! Operators can author `*.qf-task.json` files; CLI `quantforge task-run` executes them.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use thiserror::Error;

pub const TASK_GRAPH_PROTOCOL: &str = "quantforge-task-graph-v1";
pub const TASK_GRAPH_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Error)]
pub enum TaskGraphError {
    #[error("unsupported protocol `{0}`")]
    UnsupportedProtocol(String),
    #[error("unsupported schema version {0}")]
    UnsupportedSchema(u16),
    #[error("task graph has no steps")]
    Empty,
    #[error("unknown step id `{0}` referenced by depends_on")]
    UnknownDependency(String),
    #[error("cycle detected involving step `{0}`")]
    Cycle(String),
    #[error("duplicate step id `{0}`")]
    DuplicateStep(String),
    #[error("step `{0}` failed: {1}")]
    StepFailed(String, String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStepKind {
    /// Validation Challenge battery (purged WF + MC + neighborhood).
    Challenge,
    /// Fold × lookback walk-forward matrix.
    WalkForwardMatrix,
    /// M1 Judge replay.
    Judge,
    /// Generate MQL5 EA pack.
    ExportMql5,
    /// SQX-like databank expression filter (reads/writes JSON elite lists).
    DatabankFilter,
    /// No-op marker / documentation node.
    Note,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskStep {
    pub id: String,
    pub kind: TaskStepKind,
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// Opaque parameters interpreted by the runner for this kind.
    #[serde(default)]
    pub params: BTreeMap<String, Value>,
    #[serde(default)]
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskGraph {
    pub protocol: String,
    pub schema_version: u16,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub steps: Vec<TaskStep>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskStepResult {
    pub id: String,
    pub kind: TaskStepKind,
    pub status: TaskStepStatus,
    pub message: String,
    #[serde(default)]
    pub artifacts: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStepStatus {
    Passed,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskRunReport {
    pub protocol: String,
    pub graph_name: String,
    pub passed: bool,
    pub steps: Vec<TaskStepResult>,
}

impl TaskGraph {
    pub fn validate(&self) -> Result<(), TaskGraphError> {
        if self.protocol != TASK_GRAPH_PROTOCOL {
            return Err(TaskGraphError::UnsupportedProtocol(self.protocol.clone()));
        }
        if self.schema_version != TASK_GRAPH_SCHEMA_VERSION {
            return Err(TaskGraphError::UnsupportedSchema(self.schema_version));
        }
        if self.steps.is_empty() {
            return Err(TaskGraphError::Empty);
        }
        let mut seen = BTreeMap::new();
        for step in &self.steps {
            if seen.insert(step.id.clone(), ()).is_some() {
                return Err(TaskGraphError::DuplicateStep(step.id.clone()));
            }
        }
        for step in &self.steps {
            for dep in &step.depends_on {
                if !seen.contains_key(dep) {
                    return Err(TaskGraphError::UnknownDependency(dep.clone()));
                }
            }
        }
        // Kahn topological sort for cycle detection.
        let mut indegree: BTreeMap<&str, usize> =
            self.steps.iter().map(|s| (s.id.as_str(), 0usize)).collect();
        let mut edges: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for step in &self.steps {
            for dep in &step.depends_on {
                *indegree.get_mut(step.id.as_str()).unwrap() += 1;
                edges.entry(dep.as_str()).or_default().push(step.id.as_str());
            }
        }
        let mut queue: Vec<&str> = indegree
            .iter()
            .filter(|(_, d)| **d == 0)
            .map(|(id, _)| *id)
            .collect();
        let mut visited = 0usize;
        while let Some(node) = queue.pop() {
            visited += 1;
            if let Some(next) = edges.get(node) {
                for child in next {
                    let entry = indegree.get_mut(child).unwrap();
                    *entry -= 1;
                    if *entry == 0 {
                        queue.push(child);
                    }
                }
            }
        }
        if visited != self.steps.len() {
            let culprit = indegree
                .into_iter()
                .find(|(_, d)| *d > 0)
                .map(|(id, _)| id.to_string())
                .unwrap_or_else(|| "unknown".into());
            return Err(TaskGraphError::Cycle(culprit));
        }
        Ok(())
    }

    /// Return steps in dependency order (enabled steps only when `enabled_only`).
    pub fn ordered_steps(&self, enabled_only: bool) -> Result<Vec<&TaskStep>, TaskGraphError> {
        self.validate()?;
        let active: Vec<&TaskStep> = self
            .steps
            .iter()
            .filter(|step| !enabled_only || step.enabled.unwrap_or(true))
            .collect();
        let active_ids: std::collections::BTreeSet<&str> =
            active.iter().map(|step| step.id.as_str()).collect();
        let mut indegree: BTreeMap<&str, usize> =
            active.iter().map(|step| (step.id.as_str(), 0usize)).collect();
        let mut edges: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for step in &active {
            for dep in &step.depends_on {
                if !active_ids.contains(dep.as_str()) {
                    continue;
                }
                *indegree.get_mut(step.id.as_str()).unwrap() += 1;
                edges
                    .entry(dep.as_str())
                    .or_default()
                    .push(step.id.as_str());
            }
        }
        let mut queue: Vec<&str> = indegree
            .iter()
            .filter(|(_, degree)| **degree == 0)
            .map(|(id, _)| *id)
            .collect();
        let mut ordered = Vec::new();
        while let Some(id) = queue.pop() {
            let step = active.iter().find(|step| step.id == id).unwrap();
            ordered.push(*step);
            if let Some(children) = edges.get(id) {
                for child in children {
                    let degree = indegree.get_mut(child).unwrap();
                    *degree -= 1;
                    if *degree == 0 {
                        queue.push(child);
                    }
                }
            }
        }
        if ordered.len() != active.len() {
            return Err(TaskGraphError::Cycle(
                indegree
                    .into_iter()
                    .find(|(_, degree)| *degree > 0)
                    .map(|(id, _)| id.to_string())
                    .unwrap_or_else(|| "unknown".into()),
            ));
        }
        Ok(ordered)
    }
}

/// Example Retester chain used in docs / desktop primer.
pub fn example_retester_graph() -> TaskGraph {
    TaskGraph {
        protocol: TASK_GRAPH_PROTOCOL.into(),
        schema_version: TASK_GRAPH_SCHEMA_VERSION,
        name: "retester-challenge-matrix-export".into(),
        description: "Challenge → WF matrix → Judge → EA export".into(),
        steps: vec![
            TaskStep {
                id: "challenge".into(),
                kind: TaskStepKind::Challenge,
                depends_on: vec![],
                params: BTreeMap::from([
                    ("strategy".into(), Value::String("strategy.ir.json".into())),
                    ("folds".into(), Value::from(5)),
                ]),
                enabled: Some(true),
            },
            TaskStep {
                id: "wf_matrix".into(),
                kind: TaskStepKind::WalkForwardMatrix,
                depends_on: vec!["challenge".into()],
                params: BTreeMap::from([(
                    "fold_counts".into(),
                    Value::Array(vec![Value::from(3), Value::from(4), Value::from(5)]),
                )]),
                enabled: Some(true),
            },
            TaskStep {
                id: "judge".into(),
                kind: TaskStepKind::Judge,
                depends_on: vec!["challenge".into()],
                params: BTreeMap::new(),
                enabled: Some(true),
            },
            TaskStep {
                id: "export".into(),
                kind: TaskStepKind::ExportMql5,
                depends_on: vec!["judge".into()],
                params: BTreeMap::new(),
                enabled: Some(true),
            },
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn example_graph_orders_dependencies() {
        let graph = example_retester_graph();
        let ordered = graph.ordered_steps(true).unwrap();
        let ids: Vec<_> = ordered.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids[0], "challenge");
        assert!(ids.iter().position(|id| *id == "export").unwrap()
            > ids.iter().position(|id| *id == "judge").unwrap());
    }

    #[test]
    fn rejects_cycles() {
        let mut graph = example_retester_graph();
        graph.steps[0].depends_on = vec!["export".into()];
        assert!(matches!(
            graph.validate(),
            Err(TaskGraphError::Cycle(_))
        ));
    }
}
