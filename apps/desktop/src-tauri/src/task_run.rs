use quantforge_quality::{TaskGraph, TaskRunOptions, run_task_graph};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskRunRequest {
    graph_path: String,
    work_dir: String,
    #[serde(default)]
    dry_run: bool,
    #[serde(default = "default_stop_on_failure")]
    stop_on_failure: bool,
}

fn default_stop_on_failure() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskStepView {
    id: String,
    kind: String,
    status: String,
    message: String,
    artifacts: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskRunView {
    passed: bool,
    protocol: String,
    graph_name: String,
    graph_path: String,
    work_dir: String,
    dry_run: bool,
    report_path: String,
    steps: Vec<TaskStepView>,
    passed_count: usize,
    failed_count: usize,
    skipped_count: usize,
}

#[tauri::command]
pub async fn run_task_graph_workflow(request: TaskRunRequest) -> Result<TaskRunView, String> {
    tauri::async_runtime::spawn_blocking(move || run_task_graph_sync(&request))
        .await
        .map_err(|error| format!("Task-run failed: {error}"))?
}

fn run_task_graph_sync(request: &TaskRunRequest) -> Result<TaskRunView, String> {
    let graph_path = PathBuf::from(request.graph_path.trim());
    if !graph_path.is_file() {
        return Err(format!(
            "task graph does not exist: {}",
            graph_path.display()
        ));
    }
    let work_dir = PathBuf::from(request.work_dir.trim());
    if request.work_dir.trim().is_empty() {
        return Err("work directory is required".into());
    }
    fs::create_dir_all(&work_dir).map_err(|error| format!("work_dir: {error}"))?;

    let raw = fs::read_to_string(&graph_path).map_err(|error| error.to_string())?;
    let graph: TaskGraph = serde_json::from_str(&raw).map_err(|error| {
        format!(
            "invalid task graph {}: {error}",
            graph_path.display()
        )
    })?;

    let options = TaskRunOptions {
        work_dir: work_dir.clone(),
        dry_run: request.dry_run,
        stop_on_failure: request.stop_on_failure,
    };
    let report = run_task_graph(&graph, &options).map_err(|error| error.to_string())?;

    let report_path = work_dir.join("task-run-report.json");
    let pretty = serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?;
    fs::write(&report_path, pretty + "\n").map_err(|error| error.to_string())?;

    let steps: Vec<TaskStepView> = report
        .steps
        .iter()
        .map(|step| {
            let kind = serde_json::to_value(&step.kind)
                .ok()
                .and_then(|value| value.as_str().map(str::to_owned))
                .unwrap_or_else(|| format!("{:?}", step.kind));
            TaskStepView {
                id: step.id.clone(),
                kind,
                status: match step.status {
                    quantforge_quality::TaskStepStatus::Passed => "passed".into(),
                    quantforge_quality::TaskStepStatus::Failed => "failed".into(),
                    quantforge_quality::TaskStepStatus::Skipped => "skipped".into(),
                },
                message: step.message.clone(),
                artifacts: step.artifacts.clone(),
            }
        })
        .collect();

    let passed_count = steps.iter().filter(|s| s.status == "passed").count();
    let failed_count = steps.iter().filter(|s| s.status == "failed").count();
    let skipped_count = steps.iter().filter(|s| s.status == "skipped").count();

    Ok(TaskRunView {
        passed: report.passed,
        protocol: report.protocol,
        graph_name: report.graph_name,
        graph_path: graph_path.display().to_string(),
        work_dir: work_dir.display().to_string(),
        dry_run: request.dry_run,
        report_path: report_path.display().to_string(),
        steps,
        passed_count,
        failed_count,
        skipped_count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use quantforge_quality::example_retester_graph;
    use tempfile::tempdir;

    #[test]
    fn dry_runs_example_retester_graph() {
        let dir = tempdir().unwrap();
        let graph_path = dir.path().join("example.qf-task.json");
        let graph = example_retester_graph();
        fs::write(
            &graph_path,
            serde_json::to_string_pretty(&graph).unwrap() + "\n",
        )
        .unwrap();
        let work = dir.path().join("work");
        let view = run_task_graph_sync(&TaskRunRequest {
            graph_path: graph_path.display().to_string(),
            work_dir: work.display().to_string(),
            dry_run: true,
            stop_on_failure: true,
        })
        .unwrap();
        assert!(view.passed);
        assert!(view.dry_run);
        assert!(!view.steps.is_empty());
        assert!(view.steps.iter().all(|s| s.status == "passed"));
        assert!(PathBuf::from(&view.report_path).is_file());
    }
}
