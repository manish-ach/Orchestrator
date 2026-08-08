// Talks to a Forgejo (Gitea-compatible) instance on behalf of the dashboard.
// The browser never calls Forgejo directly — the coordinator fetches repo
// metadata here and serves it from /api/repos. Set FORGEJO_TOKEN for
// private repos; public repos need no auth.

use reqwest::Client;
use serde_json::Value;
use crate::types::{Contributor, LanguageShare, PipelineJobRef, PipelineRef, Repo};

/// Files probed on the default branch to detect a pipeline definition,
/// in priority order.
pub const PIPELINE_FILES: [&str; 4] =
    [".orchestrator/actions.yml", ".orchestrator/ci.yml", "pipeline.yml", "pipeline.yaml"];

/// How many recent commits to scan for contributor names.
const COMMIT_SCAN_LIMIT: u8 = 30;

pub struct RemoteRef {
    pub base: String,
    pub owner: String,
    pub name: String,
}

/// `https://git.example.com/Owner/Repo[.git][/]` -> base + owner + name
pub fn parse_remote(remote: &str) -> Option<RemoteRef> {
    let url = remote.trim().trim_end_matches('/').trim_end_matches(".git");
    let scheme = if url.starts_with("http://") { "http" } else { "https" };
    let rest = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://"))?;
    let mut parts = rest.split('/');
    let host = parts.next()?;
    let owner = parts.next()?;
    let name = parts.next()?;
    if host.is_empty() || owner.is_empty() || name.is_empty() {
        return None;
    }
    Some(RemoteRef {
        base: format!("{scheme}://{host}"),
        owner: owner.to_string(),
        name: name.to_string(),
    })
}

async fn get_json(client: &Client, url: &str) -> Option<Value> {
    let mut req = client.get(url).header("Accept", "application/json");
    if let Ok(token) = std::env::var("FORGEJO_TOKEN") {
        req = req.header("Authorization", format!("token {token}"));
    }
    let resp = req.send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    resp.json().await.ok()
}

pub async fn fetch_repo(client: &Client, remote: &str) -> Result<Repo, String> {
    let r = parse_remote(remote)
        .ok_or_else(|| format!("'{remote}' is not a valid repo URL (expected https://host/owner/repo)"))?;
    let api = format!("{}/api/v1/repos/{}/{}", r.base, r.owner, r.name);

    let info = get_json(client, &api).await.ok_or_else(|| {
        format!("Forgejo at {} returned no repo info for {}/{}", r.base, r.owner, r.name)
    })?;

    // bytes per language -> percentages, largest first
    let mut languages: Vec<LanguageShare> = Vec::new();
    if let Some(map) = get_json(client, &format!("{api}/languages"))
        .await
        .and_then(|v| v.as_object().cloned())
    {
        let total: f64 = map.values().filter_map(Value::as_f64).sum();
        if total > 0.0 {
            languages = map
                .iter()
                .filter_map(|(name, bytes)| bytes.as_f64().map(|b| (name.clone(), b)))
                .map(|(name, b)| LanguageShare { name, pct: (b / total * 1000.0).round() / 10.0 })
                .collect();
            languages.sort_by(|a, b| b.pct.total_cmp(&a.pct));
        }
    }

    // unique authors of the latest commits (Forgejo has no /contributors endpoint)
    let mut contributors: Vec<Contributor> = Vec::new();
    let commits_url = format!("{api}/commits?limit={COMMIT_SCAN_LIMIT}&stat=false");
    if let Some(commits) = get_json(client, &commits_url).await.and_then(|v| v.as_array().cloned()) {
        for c in &commits {
            let name = c
                .pointer("/commit/author/name")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_string();
            let login = c
                .pointer("/author/login")
                .and_then(Value::as_str)
                .unwrap_or(&name)
                .to_string();
            if !contributors.iter().any(|x| x.login == login) {
                contributors.push(Contributor { login, name });
            }
        }
    }

    // name the pipeline after the YAML's `name:` so runs (created from the
    // same file) group under it in the dashboard's pipeline switcher
    let branch = info["default_branch"].as_str().unwrap_or("main");
    // Every candidate file, not just the first: a repo with both a ci.yml and
    // a nightly.yml has two pipelines, and stopping at one hid the second
    // until it happened to run.
    let mut pipelines: Vec<PipelineRef> = Vec::new();
    for file in PIPELINE_FILES {
        let Some(yaml) = fetch_raw_file(client, remote, branch, file).await else { continue };
        let name = crate::pipeline::yaml_pipeline_name(&yaml).unwrap_or_else(|| format!("{}-ci", r.name));
        // Planned through the real parser rather than a local guess, so the
        // shape shown here is exactly the shape that will run — and a file that
        // does not validate says so instead of rendering half a graph.
        let (stages, jobs, parse_error) = match crate::pipeline::plan_from_yaml(&yaml).await {
            Ok(plan) => (
                plan.stages,
                plan.jobs
                    .into_iter()
                    .map(|j| PipelineJobRef { name: j.name, stage: j.stage, needs: j.needs, tags: j.tags })
                    .collect(),
                None,
            ),
            Err(e) => (Vec::new(), Vec::new(), Some(e)),
        };
        let schedule = crate::pipeline::yaml_schedule(&yaml);
        pipelines.push(PipelineRef { name, file: file.to_string(), stages, jobs, parse_error, schedule });
    }

    Ok(Repo {
        name: info["name"].as_str().unwrap_or(&r.name).to_string(),
        description: info["description"].as_str().unwrap_or("").to_string(),
        language: info["language"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .or_else(|| languages.first().map(|l| l.name.clone()))
            .unwrap_or_else(|| "—".to_string()),
        branch: info["default_branch"].as_str().unwrap_or("main").to_string(),
        owner: info.pointer("/owner/login").and_then(Value::as_str).unwrap_or(&r.owner).to_string(),
        remote: Some(remote.trim().trim_end_matches('/').to_string()),
        languages,
        contributors,
        pipelines,
        // joined in by the store on read, not sourced from Forgejo
        webhook: None,
    })
}

/// Fetch a file's raw content from a repo at a given ref. Returns None if
/// the URL doesn't parse or the file is absent on that ref.
pub async fn fetch_raw_file(client: &Client, remote: &str, branch: &str, path: &str) -> Option<String> {
    let r = parse_remote(remote)?;
    let url = format!(
        "{}/api/v1/repos/{}/{}/raw/{}?ref={}",
        r.base, r.owner, r.name, path, branch
    );

    let mut req = client.get(&url);
    if let Ok(token) = std::env::var("FORGEJO_TOKEN") {
        req = req.header("Authorization", format!("token {token}"));
    }
    let resp = req.send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    resp.text().await.ok()
}

#[cfg(test)]
mod tests {
    use super::parse_remote;

    /// The webhook matcher resolves a repo by comparing owner/name from the
    /// registered remote against the payload's `full_name`. Every one of these
    /// spellings must land on the same pair, because a user can register any
    /// of them and Forgejo always sends the bare "owner/name".
    #[test]
    fn remote_spellings_all_resolve_to_the_same_repo() {
        let expected = ("Manish", "orchestrator-run-test");
        for remote in [
            "https://git.example.com/Manish/orchestrator-run-test",
            "https://git.example.com/Manish/orchestrator-run-test/",
            "https://git.example.com/Manish/orchestrator-run-test.git",
            "  https://git.example.com/Manish/orchestrator-run-test.git  ",
            "http://git.example.com/Manish/orchestrator-run-test",
        ] {
            let r = parse_remote(remote).unwrap_or_else(|| panic!("failed to parse {remote}"));
            assert_eq!((r.owner.as_str(), r.name.as_str()), expected, "for {remote}");
        }
    }

    #[test]
    fn rejects_urls_that_are_not_a_repo() {
        for bad in ["", "not-a-url", "https://git.example.com", "https://git.example.com/owner"] {
            assert!(parse_remote(bad).is_none(), "{bad} should not parse");
        }
    }
}
