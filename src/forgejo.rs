// Talks to a Forgejo (Gitea-compatible) instance on behalf of the dashboard.
// The browser never calls Forgejo directly — the coordinator fetches repo
// metadata here and serves it from /api/repos. Set FORGEJO_TOKEN for
// private repos; public repos need no auth.

use reqwest::Client;
use serde_json::Value;
use crate::types::{Contributor, LanguageShare, PipelineRef, Repo};

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
    let mut pipelines: Vec<PipelineRef> = Vec::new();
    for file in PIPELINE_FILES {
        if let Some(yaml) = fetch_raw_file(client, remote, branch, file).await {
            let name = crate::pipeline::yaml_pipeline_name(&yaml).unwrap_or_else(|| format!("{}-ci", r.name));
            pipelines.push(PipelineRef { name, file: file.to_string() });
            break;
        }
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
