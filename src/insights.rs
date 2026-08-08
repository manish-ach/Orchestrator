// Aggregates for the Insights page.
//
// Everything here is a pure function over rows the store already has. The
// coordinator fetches one flat job×run join for the window and derives the
// whole page from it in memory — at this project's scale that is cheaper than
// eight round trips to Postgres, and it makes every number testable without a
// database.
//
// The unit of analysis is what separates this page from the rest of the site:
// Control center and Runs are keyed by run id, so anything keyed by a *stage*,
// *job name*, *worker* or *time bucket* belongs here and only here.

use std::collections::HashMap;

use serde::Serialize;

use crate::types::{CalendarDay, JobStatus};

/// One job, flattened with the run it belonged to. The shape the store hands
/// this module, and the only input any function below takes.
#[derive(Debug, Clone)]
pub struct Row {
    pub run_id: i64,
    pub pipeline: String,
    pub repo: String,
    pub commit_sha: Option<String>,
    pub run_status: JobStatus,
    pub run_started_at: i64,
    pub run_finished_at: Option<i64>,
    pub stage: String,
    pub job_name: String,
    pub job_status: JobStatus,
    pub ready_at: Option<i64>,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub exit_code: Option<i32>,
    pub first_error: Option<String>,
    pub worker: Option<String>,
}

/// Where a run's time actually goes, per stage.
#[derive(Debug, Clone, Serialize)]
pub struct StageCost {
    pub stage: String,
    /// distinct job names seen in this stage
    pub jobs: i64,
    /// median stage wall time — last finish minus first start within a run
    pub median_ms: i64,
    pub p90_ms: i64,
    /// runs this stage appeared in
    pub runs: i64,
}

/// A job that both passed and failed on the same commit.
#[derive(Debug, Clone, Serialize)]
pub struct FlakyJob {
    pub name: String,
    pub repo: String,
    /// commits where this job produced both outcomes
    pub flips: i64,
    /// most recent outcomes, oldest first — 'p' passed, 'f' failed
    pub recent: String,
}

/// One day of run durations.
#[derive(Debug, Clone, Serialize)]
pub struct TrendPoint {
    pub date: String,
    pub runs: i64,
    pub p50_ms: i64,
    pub p90_ms: i64,
}

/// One day split into time spent waiting for a worker versus executing.
#[derive(Debug, Clone, Serialize)]
pub struct WaitPoint {
    pub date: String,
    pub wait_ms: i64,
    pub exec_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkerCost {
    pub worker: String,
    pub runs: i64,
    pub median_ms: i64,
    /// 0–100, or None when nothing finished on this worker
    pub pass_pct: Option<i64>,
}

/// How one job behaves across machines — the case for a `tags:` pin.
#[derive(Debug, Clone, Serialize)]
pub struct JobSkew {
    pub job: String,
    pub workers: Vec<WorkerCost>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FailureCause {
    /// the distilled first error line, as stored
    pub cause: String,
    pub job: String,
    pub exit_code: Option<i32>,
    pub count: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Insights {
    pub range_days: i64,
    pub runs: i64,
    pub passed: i64,
    pub failed: i64,
    pub median_ms: Option<i64>,
    pub p90_ms: Option<i64>,
    /// median time from a pipeline failing to its next pass
    pub recovery_ms: Option<i64>,
    /// longest unbroken stretch a pipeline spent failing
    pub longest_red_ms: Option<i64>,
    /// a year of daily run counts — wider than `range_days` on purpose: the
    /// question it answers ("when does this fleet get used") needs a year
    pub calendar: Vec<CalendarDay>,
    pub stages: Vec<StageCost>,
    pub flaky: Vec<FlakyJob>,
    pub trend: Vec<TrendPoint>,
    pub wait: Vec<WaitPoint>,
    pub skew: Option<JobSkew>,
    pub causes: Vec<FailureCause>,
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

/// Nearest-rank percentile of an already-sorted slice. `None` when empty, so a
/// caller can say "not enough data" rather than print a zero it invented.
fn pct(sorted: &[i64], p: f64) -> Option<i64> {
    if sorted.is_empty() {
        return None;
    }
    let rank = ((p / 100.0) * sorted.len() as f64).ceil() as usize;
    Some(sorted[rank.saturating_sub(1).min(sorted.len() - 1)])
}

fn median(sorted: &[i64]) -> Option<i64> {
    pct(sorted, 50.0)
}

fn day_key(ms: i64) -> String {
    use chrono::{Local, TimeZone};
    Local
        .timestamp_millis_opt(ms)
        .single()
        .map(|d| d.format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

/// One entry per run in the window, since most run-level questions want each
/// run once rather than once per job.
#[derive(Debug, Clone)]
struct RunView {
    pipeline: String,
    status: JobStatus,
    started_at: i64,
    finished_at: Option<i64>,
}

fn runs_of(rows: &[Row]) -> Vec<RunView> {
    let mut seen: HashMap<i64, RunView> = HashMap::new();
    for r in rows {
        seen.entry(r.run_id).or_insert_with(|| RunView {
            pipeline: r.pipeline.clone(),
            status: r.run_status.clone(),
            started_at: r.run_started_at,
            finished_at: r.run_finished_at,
        });
    }
    let mut out: Vec<RunView> = seen.into_values().collect();
    out.sort_by_key(|r| r.started_at);
    out
}

fn run_durations(runs: &[RunView]) -> Vec<i64> {
    let mut d: Vec<i64> = runs
        .iter()
        .filter_map(|r| r.finished_at.map(|f| f - r.started_at))
        .filter(|&ms| ms >= 0)
        .collect();
    d.sort_unstable();
    d
}

// ---------------------------------------------------------------------------
// panels
// ---------------------------------------------------------------------------

/// Median time from a pipeline going red to its next green.
///
/// Measured per pipeline, in start order: a failure is only "recovered" by a
/// later pass of the *same* pipeline. A pipeline still red at the end of the
/// window contributes nothing rather than a censored number — an open incident
/// is not a fast recovery.
fn recovery_ms(runs: &[RunView]) -> Option<i64> {
    let mut by_pipe: HashMap<&str, Vec<&RunView>> = HashMap::new();
    for r in runs {
        by_pipe.entry(&r.pipeline).or_default().push(r);
    }
    let mut spans = Vec::new();
    for list in by_pipe.values() {
        let mut broke_at: Option<i64> = None;
        for r in list {
            match r.status {
                JobStatus::Failed => {
                    if broke_at.is_none() {
                        broke_at = Some(r.finished_at.unwrap_or(r.started_at));
                    }
                }
                JobStatus::Passed => {
                    if let Some(t) = broke_at.take() {
                        let fixed = r.finished_at.unwrap_or(r.started_at);
                        if fixed >= t {
                            spans.push(fixed - t);
                        }
                    }
                }
                _ => {}
            }
        }
    }
    spans.sort_unstable();
    median(&spans)
}

/// Longest unbroken stretch any one pipeline spent failing. Same walk as
/// `recovery_ms`, but the maximum rather than the median, and a pipeline still
/// red at the end counts up to `now` — that streak is real and still running.
fn longest_red_ms(runs: &[RunView], now: i64) -> Option<i64> {
    let mut by_pipe: HashMap<&str, Vec<&RunView>> = HashMap::new();
    for r in runs {
        by_pipe.entry(&r.pipeline).or_default().push(r);
    }
    let mut longest: Option<i64> = None;
    for list in by_pipe.values() {
        let mut broke_at: Option<i64> = None;
        for r in list {
            match r.status {
                JobStatus::Failed if broke_at.is_none() => {
                    broke_at = Some(r.finished_at.unwrap_or(r.started_at));
                }
                JobStatus::Passed => {
                    if let Some(t) = broke_at.take() {
                        let span = r.finished_at.unwrap_or(r.started_at) - t;
                        longest = Some(longest.map_or(span, |l: i64| l.max(span)));
                    }
                }
                _ => {}
            }
        }
        if let Some(t) = broke_at {
            let span = now - t;
            longest = Some(longest.map_or(span, |l: i64| l.max(span)));
        }
    }
    longest.filter(|&ms| ms > 0)
}

/// Per-stage wall time, so the answer to "which stage should we attack" is the
/// stage's own elapsed time and not the sum of jobs that ran in parallel.
pub fn stage_costs(rows: &[Row]) -> Vec<StageCost> {
    // (run, stage) -> (earliest start, latest finish)
    let mut spans: HashMap<(i64, &str), (i64, i64)> = HashMap::new();
    let mut names: HashMap<&str, std::collections::HashSet<&str>> = HashMap::new();
    for r in rows {
        names.entry(&r.stage).or_default().insert(&r.job_name);
        let (Some(s), Some(f)) = (r.started_at, r.finished_at) else { continue };
        if f < s {
            continue;
        }
        spans
            .entry((r.run_id, &r.stage))
            .and_modify(|e| {
                e.0 = e.0.min(s);
                e.1 = e.1.max(f);
            })
            .or_insert((s, f));
    }

    let mut by_stage: HashMap<&str, Vec<i64>> = HashMap::new();
    for ((_, stage), (s, f)) in spans {
        by_stage.entry(stage).or_default().push(f - s);
    }

    let mut out: Vec<StageCost> = by_stage
        .into_iter()
        .map(|(stage, mut d)| {
            d.sort_unstable();
            StageCost {
                stage: stage.to_string(),
                jobs: names.get(stage).map(|s| s.len()).unwrap_or(0) as i64,
                median_ms: median(&d).unwrap_or(0),
                p90_ms: pct(&d, 90.0).unwrap_or(0),
                runs: d.len() as i64,
            }
        })
        .collect();
    out.sort_by(|a, b| b.median_ms.cmp(&a.median_ms));
    out
}

/// Jobs that both passed and failed on the same commit.
///
/// Same commit is the whole point: a job that fails on a broken commit is doing
/// its job. A job that disagrees with itself about identical code is flaky, and
/// no amount of re-running will tell you which answer was right.
pub fn flaky_jobs(rows: &[Row]) -> Vec<FlakyJob> {
    struct Acc {
        repo: String,
        /// sha -> (passed?, failed?)
        by_sha: HashMap<String, (bool, bool)>,
        /// outcomes in start order
        recent: Vec<(i64, char)>,
    }
    let mut acc: HashMap<&str, Acc> = HashMap::new();
    for r in rows {
        let Some(sha) = r.commit_sha.as_ref() else { continue };
        let mark = match r.job_status {
            JobStatus::Passed => 'p',
            JobStatus::Failed => 'f',
            _ => continue,
        };
        let e = acc.entry(&r.job_name).or_insert_with(|| Acc {
            repo: r.repo.clone(),
            by_sha: HashMap::new(),
            recent: Vec::new(),
        });
        let slot = e.by_sha.entry(sha.clone()).or_insert((false, false));
        if mark == 'p' {
            slot.0 = true;
        } else {
            slot.1 = true;
        }
        e.recent.push((r.started_at.unwrap_or(0), mark));
    }

    let mut out: Vec<FlakyJob> = acc
        .into_iter()
        .filter_map(|(name, mut e)| {
            let flips = e.by_sha.values().filter(|(p, f)| *p && *f).count() as i64;
            if flips == 0 {
                return None;
            }
            e.recent.sort_by_key(|(t, _)| *t);
            let recent: String = e.recent.iter().rev().take(20).rev().map(|(_, c)| *c).collect();
            Some(FlakyJob { name: name.to_string(), repo: e.repo, flips, recent })
        })
        .collect();
    out.sort_by(|a, b| b.flips.cmp(&a.flips));
    out.truncate(6);
    out
}

/// Daily p50 and p90 of run duration. Days with no finished run are omitted
/// rather than plotted as zero — a gap is honest, a zero is a lie about a fast
/// day that never happened.
fn duration_trend(runs: &[RunView]) -> Vec<TrendPoint> {
    let mut by_day: HashMap<String, Vec<i64>> = HashMap::new();
    for r in runs {
        let Some(f) = r.finished_at else { continue };
        if f < r.started_at {
            continue;
        }
        by_day.entry(day_key(r.started_at)).or_default().push(f - r.started_at);
    }
    let mut out: Vec<TrendPoint> = by_day
        .into_iter()
        .map(|(date, mut d)| {
            d.sort_unstable();
            TrendPoint {
                date,
                runs: d.len() as i64,
                p50_ms: median(&d).unwrap_or(0),
                p90_ms: pct(&d, 90.0).unwrap_or(0),
            }
        })
        .collect();
    out.sort_by(|a, b| a.date.cmp(&b.date));
    out
}

/// Daily queue wait against execution time, summed over jobs.
///
/// This is the "more workers or faster workers?" question, and it is only
/// answerable because `ready_at` records when a job *could* have started.
pub fn queue_wait(rows: &[Row]) -> Vec<WaitPoint> {
    let mut by_day: HashMap<String, (i64, i64)> = HashMap::new();
    for r in rows {
        let Some(started) = r.started_at else { continue };
        let e = by_day.entry(day_key(started)).or_insert((0, 0));
        if let Some(ready) = r.ready_at {
            if started > ready {
                e.0 += started - ready;
            }
        }
        if let Some(f) = r.finished_at {
            if f > started {
                e.1 += f - started;
            }
        }
    }
    let mut out: Vec<WaitPoint> = by_day
        .into_iter()
        .map(|(date, (wait_ms, exec_ms))| WaitPoint { date, wait_ms, exec_ms })
        .collect();
    out.sort_by(|a, b| a.date.cmp(&b.date));
    out
}

/// How the most-executed job behaves per machine. One job, because the
/// comparison is only meaningful within a job — "worker A is slower" across
/// different work is a statement about the schedule, not the hardware.
pub fn worker_skew(rows: &[Row]) -> Option<JobSkew> {
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for r in rows {
        if r.worker.is_some() && r.finished_at.is_some() {
            *counts.entry(&r.job_name).or_default() += 1;
        }
    }
    let (job, _) = counts.into_iter().max_by_key(|&(name, n)| (n, std::cmp::Reverse(name)))?;

    let mut per: HashMap<&str, (Vec<i64>, i64, i64)> = HashMap::new();
    for r in rows.iter().filter(|r| r.job_name == job) {
        let Some(w) = r.worker.as_deref() else { continue };
        let e = per.entry(w).or_insert((Vec::new(), 0, 0));
        match r.job_status {
            JobStatus::Passed => e.1 += 1,
            JobStatus::Failed => e.2 += 1,
            _ => {}
        }
        if let (Some(s), Some(f)) = (r.started_at, r.finished_at) {
            if f >= s {
                e.0.push(f - s);
            }
        }
    }

    let mut workers: Vec<WorkerCost> = per
        .into_iter()
        .filter(|(_, (d, _, _))| !d.is_empty())
        .map(|(w, (mut d, ok, bad))| {
            d.sort_unstable();
            WorkerCost {
                worker: w.to_string(),
                runs: d.len() as i64,
                median_ms: median(&d).unwrap_or(0),
                pass_pct: (ok + bad > 0).then(|| (ok * 100) / (ok + bad)),
            }
        })
        .collect();
    if workers.len() < 2 {
        // one machine is not a comparison
        return None;
    }
    workers.sort_by_key(|w| w.median_ms);
    Some(JobSkew { job: job.to_string(), workers })
}

/// Failures ranked by cause, using the line the coordinator distilled on
/// report. Grouped by (cause, job) so the same message from two different jobs
/// stays two entries — they are two different bugs.
pub fn failure_causes(rows: &[Row]) -> Vec<FailureCause> {
    let mut acc: HashMap<(&str, &str), (Option<i32>, i64)> = HashMap::new();
    for r in rows.iter().filter(|r| r.job_status == JobStatus::Failed) {
        let cause = r.first_error.as_deref().unwrap_or("no output captured");
        let e = acc.entry((cause, &r.job_name)).or_insert((r.exit_code, 0));
        e.1 += 1;
    }
    let mut out: Vec<FailureCause> = acc
        .into_iter()
        .map(|((cause, job), (exit_code, count))| FailureCause {
            cause: cause.to_string(),
            job: job.to_string(),
            exit_code,
            count,
        })
        .collect();
    out.sort_by(|a, b| b.count.cmp(&a.count).then(a.cause.cmp(&b.cause)));
    out.truncate(8);
    out
}

/// Assemble the whole page from one window of rows.
pub fn build(rows: &[Row], range_days: i64, now: i64, calendar: Vec<CalendarDay>) -> Insights {
    let runs = runs_of(rows);
    let durations = run_durations(&runs);
    Insights {
        range_days,
        runs: runs.len() as i64,
        passed: runs.iter().filter(|r| r.status == JobStatus::Passed).count() as i64,
        failed: runs.iter().filter(|r| r.status == JobStatus::Failed).count() as i64,
        median_ms: median(&durations),
        p90_ms: pct(&durations, 90.0),
        recovery_ms: recovery_ms(&runs),
        longest_red_ms: longest_red_ms(&runs, now),
        calendar,
        stages: stage_costs(rows),
        flaky: flaky_jobs(rows),
        trend: duration_trend(&runs),
        wait: queue_wait(rows),
        skew: worker_skew(rows),
        causes: failure_causes(rows),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(run: i64, name: &str, status: JobStatus) -> Row {
        Row {
            run_id: run,
            pipeline: "ci".into(),
            repo: "r".into(),
            commit_sha: Some("abc".into()),
            run_status: status.clone(),
            run_started_at: run * 1000,
            run_finished_at: Some(run * 1000 + 500),
            stage: "test".into(),
            job_name: name.into(),
            job_status: status,
            ready_at: Some(run * 1000),
            started_at: Some(run * 1000 + 100),
            finished_at: Some(run * 1000 + 400),
            exit_code: Some(1),
            first_error: Some("boom".into()),
            worker: Some("w1".into()),
        }
    }

    #[test]
    fn percentile_uses_nearest_rank_and_never_indexes_past_the_end() {
        let d = [10, 20, 30, 40];
        assert_eq!(median(&d), Some(20));
        assert_eq!(pct(&d, 90.0), Some(40));
        assert_eq!(pct(&d, 100.0), Some(40));
        assert_eq!(pct(&[], 50.0), None);
        assert_eq!(pct(&[7], 90.0), Some(7));
    }

    #[test]
    fn a_stage_costs_its_wall_time_not_the_sum_of_parallel_jobs() {
        // two jobs in one stage, overlapping: 0-300 and 100-400 → 400ms elapsed,
        // not the 600ms you get by adding them
        let mut a = row(1, "x", JobStatus::Passed);
        a.started_at = Some(0);
        a.finished_at = Some(300);
        let mut b = row(1, "y", JobStatus::Passed);
        b.started_at = Some(100);
        b.finished_at = Some(400);
        let out = stage_costs(&[a, b]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].median_ms, 400);
        assert_eq!(out[0].jobs, 2, "two distinct job names in the stage");
    }

    #[test]
    fn flaky_needs_both_outcomes_on_one_commit() {
        // same job, same sha, disagreeing → flaky
        let pass = row(1, "unit", JobStatus::Passed);
        let fail = row(2, "unit", JobStatus::Failed);
        let out = flaky_jobs(&[pass, fail]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].flips, 1);
        assert_eq!(out[0].recent, "pf");
    }

    #[test]
    fn a_job_that_only_ever_fails_is_broken_not_flaky() {
        let a = row(1, "unit", JobStatus::Failed);
        let b = row(2, "unit", JobStatus::Failed);
        assert!(flaky_jobs(&[a, b]).is_empty());
    }

    #[test]
    fn failing_on_different_commits_is_not_flaky() {
        let mut a = row(1, "unit", JobStatus::Passed);
        a.commit_sha = Some("aaa".into());
        let mut b = row(2, "unit", JobStatus::Failed);
        b.commit_sha = Some("bbb".into());
        assert!(flaky_jobs(&[a, b]).is_empty());
    }

    #[test]
    fn recovery_measures_red_to_the_next_green_of_the_same_pipeline() {
        let mut broke = row(1, "x", JobStatus::Failed);
        broke.run_finished_at = Some(1_000);
        let mut other = row(2, "x", JobStatus::Passed);
        other.pipeline = "nightly".into(); // a different pipeline must not "fix" it
        other.run_finished_at = Some(2_000);
        let mut fixed = row(3, "x", JobStatus::Passed);
        fixed.run_finished_at = Some(5_000);
        assert_eq!(recovery_ms(&runs_of(&[broke, other, fixed])), Some(4_000));
    }

    #[test]
    fn a_pipeline_still_red_reports_a_streak_but_no_recovery() {
        let mut broke = row(1, "x", JobStatus::Failed);
        broke.run_finished_at = Some(1_000);
        let runs = runs_of(&[broke]);
        assert_eq!(recovery_ms(&runs), None, "an open incident is not a fast recovery");
        assert_eq!(longest_red_ms(&runs, 9_000), Some(8_000));
    }

    #[test]
    fn queue_wait_is_ready_to_start_and_never_negative() {
        let mut r = row(1, "x", JobStatus::Passed);
        r.ready_at = Some(1_000);
        r.started_at = Some(1_700);
        r.finished_at = Some(3_000);
        let out = queue_wait(&[r]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].wait_ms, 700);
        assert_eq!(out[0].exec_ms, 1_300);
    }

    #[test]
    fn a_job_that_ran_on_one_machine_is_not_a_skew_comparison() {
        let a = row(1, "unit", JobStatus::Passed);
        let b = row(2, "unit", JobStatus::Passed);
        assert!(worker_skew(&[a, b]).is_none());
    }

    #[test]
    fn skew_ranks_machines_by_median_for_the_busiest_job() {
        let mut fast = row(1, "unit", JobStatus::Passed);
        fast.worker = Some("beefy".into());
        fast.started_at = Some(0);
        fast.finished_at = Some(100);
        let mut slow = row(2, "unit", JobStatus::Passed);
        slow.worker = Some("laptop".into());
        slow.started_at = Some(0);
        slow.finished_at = Some(900);
        let skew = worker_skew(&[fast, slow]).expect("two machines ran it");
        assert_eq!(skew.job, "unit");
        assert_eq!(skew.workers[0].worker, "beefy");
        assert_eq!(skew.workers[0].median_ms, 100);
        assert_eq!(skew.workers[1].median_ms, 900);
    }

    #[test]
    fn the_same_message_from_two_jobs_stays_two_causes() {
        let mut a = row(1, "migrate", JobStatus::Failed);
        a.first_error = Some("db not seeded".into());
        let mut b = row(2, "integration", JobStatus::Failed);
        b.first_error = Some("db not seeded".into());
        let out = failure_causes(&[a, b]);
        assert_eq!(out.len(), 2, "one message, two bugs");
    }

    #[test]
    fn causes_rank_by_count() {
        let mut one = row(1, "migrate", JobStatus::Failed);
        one.first_error = Some("rare".into());
        let mut a = row(2, "unit", JobStatus::Failed);
        a.first_error = Some("common".into());
        let b = a.clone();
        let out = failure_causes(&[one, a, b]);
        assert_eq!(out[0].cause, "common");
        assert_eq!(out[0].count, 2);
    }

    #[test]
    fn an_empty_window_reports_nothing_rather_than_zeroes() {
        let i = build(&[], 30, 1_000, vec![]);
        assert_eq!(i.runs, 0);
        assert_eq!(i.median_ms, None);
        assert_eq!(i.p90_ms, None);
        assert_eq!(i.recovery_ms, None);
        assert!(i.stages.is_empty() && i.flaky.is_empty() && i.causes.is_empty());
        assert!(i.skew.is_none());
    }
}
