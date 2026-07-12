// Bridge to the Python yaml-parser. The coordinator hands pipeline YAML to
// yaml-parser/plan_json.py, which validates it (schema, stages, needs,
// cycles) and returns the jobs in execution order.
//
//   YAML_PARSER_DIR     — where the parser lives (default: yaml-parser)
//   YAML_PARSER_PYTHON  — python used to run it (default: the parser's
//                         .venv python if present, else python3)

use std::collections::HashMap;
use std::io::Write;
use std::path::Path;

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct PlanJob {
    pub name: String,
    pub stage: String,
    pub command: String,
    #[serde(default)]
    pub needs: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// workspace paths to upload as artifacts when the job passes
    #[serde(default)]
    pub artifacts: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Plan {
    pub name: String,
    pub stages: Vec<String>,
    pub jobs: Vec<PlanJob>,
}

#[derive(Deserialize)]
struct ParserError {
    error: String,
}

fn parser_dir() -> String {
    std::env::var("YAML_PARSER_DIR").unwrap_or_else(|_| "yaml-parser".into())
}

fn parser_python() -> String {
    if let Ok(py) = std::env::var("YAML_PARSER_PYTHON") {
        return py;
    }
    let venv = format!("{}/.venv/bin/python", parser_dir());
    if Path::new(&venv).exists() { venv } else { "python3".into() }
}

/// Validate + plan pipeline YAML by invoking the Python parser.
pub async fn plan_from_yaml(content: &str) -> Result<Plan, String> {
    let mut tmp = tempfile::NamedTempFile::new().map_err(|e| format!("temp file: {e}"))?;
    tmp.write_all(content.as_bytes()).map_err(|e| format!("temp file: {e}"))?;

    let script = format!("{}/plan_json.py", parser_dir());
    let out = tokio::process::Command::new(parser_python())
        .arg(&script)
        .arg(tmp.path())
        .output()
        .await
        .map_err(|e| format!("could not run yaml-parser ({script}): {e}"))?;

    let stdout = String::from_utf8_lossy(&out.stdout);
    if let Ok(err) = serde_json::from_str::<ParserError>(&stdout) {
        return Err(format!("pipeline YAML invalid: {}", err.error));
    }
    serde_json::from_str::<Plan>(&stdout).map_err(|e| {
        let stderr = String::from_utf8_lossy(&out.stderr);
        format!("yaml-parser returned unexpected output: {e} — {stdout} {stderr}")
    })
}

/// Extract the pipeline `name:` from raw YAML without invoking the parser —
/// used to label PipelineRefs so the dashboard can match runs to pipelines.
pub fn yaml_pipeline_name(yaml: &str) -> Option<String> {
    yaml.lines()
        .find(|l| l.starts_with("name:"))
        .map(|l| l.trim_start_matches("name:").trim().trim_matches(['"', '\'']).to_string())
        .filter(|s| !s.is_empty())
}
