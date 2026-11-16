//! Problem lifecycle: a problem is (sensor, check); it opens when the check fires and closes
//! only on positive evidence.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde::{Deserialize, Serialize};

use super::checks::{Check, Ctx, Evidence, Reading, evaluate, physical_range};

/// One open (or just closed) problem.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Problem {
    pub entity_id: String,
    pub name: String,
    pub check: Check,
    pub opened_at: u64,
    pub evidence: Evidence,
    /// The push that announced it, if one was sent.
    pub alert_id: Option<String>,
}

impl Problem {
    /// The id of this problem's own (single, not grouped) opened alert. Derived, so a Mute
    /// reply resolves even for problems loaded after a restart.
    pub fn alert_key(&self) -> String {
        format!(
            "supervisor-{}-{}-{}",
            self.check.as_str(),
            self.entity_id,
            self.opened_at
        )
    }

    /// Whether a silent problem was opened by the offline trigger (no baseline involved).
    fn opened_offline(&self) -> bool {
        self.check == Check::Silent && self.evidence.usual_secs.is_none()
    }
}

/// A problem that closed this run.
#[derive(Debug, Clone, PartialEq)]
pub struct Closed {
    pub problem: Problem,
    pub at: u64,
    /// The entity vanished from the registry: closed without a push.
    pub silent: bool,
}

/// What one run changed.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Step {
    pub opened: Vec<Problem>,
    pub closed: Vec<Closed>,
}

/// The open problems and how they move from run to run.
#[derive(Debug, Default)]
pub struct Supervisor {
    problems: BTreeMap<(String, Check), Problem>,
}

impl Supervisor {
    /// A supervisor resuming the given open problems (loaded from storage).
    pub fn new(open: Vec<Problem>) -> Self {
        let problems = open
            .into_iter()
            .map(|p| ((p.entity_id.clone(), p.check), p))
            .collect();
        Self { problems }
    }

    /// The open problems.
    pub fn open(&self) -> impl Iterator<Item = &Problem> {
        self.problems.values()
    }

    /// One run: close the problems with positive evidence, open the new ones.
    pub fn step(&mut self, readings: &[Reading], ctx: &Ctx) -> Step {
        let by_id: HashMap<&str, &Reading> =
            readings.iter().map(|r| (r.entity_id.as_str(), r)).collect();
        let mut step = Step::default();
        let mut just_closed = BTreeSet::new();

        let keys: Vec<(String, Check)> = self.problems.keys().cloned().collect();
        for key in keys {
            let p = &self.problems[&key];
            let silent = match by_id.get(p.entity_id.as_str()) {
                None => true,
                Some(r) if recovered(p, r, ctx) => false,
                Some(_) => continue,
            };
            let problem = self.problems.remove(&key).expect("key listed above");
            step.closed.push(Closed {
                problem,
                at: ctx.now,
                silent,
            });
            just_closed.insert(key);
        }

        for r in readings {
            for finding in evaluate(r, ctx) {
                let key = (r.entity_id.clone(), finding.check);
                if self.problems.contains_key(&key) || just_closed.contains(&key) {
                    continue;
                }
                let problem = Problem {
                    entity_id: r.entity_id.clone(),
                    name: r.name.clone(),
                    check: finding.check,
                    opened_at: ctx.now,
                    evidence: finding.evidence,
                    alert_id: None,
                };
                step.opened.push(problem.clone());
                self.problems.insert(key, problem);
            }
        }
        step
    }

    /// Drops the entity's open problems (no push) and returns them.
    pub fn mute(&mut self, entity_id: &str) -> Vec<Problem> {
        let keys: Vec<(String, Check)> = self
            .problems
            .keys()
            .filter(|(id, _)| id == entity_id)
            .cloned()
            .collect();
        keys.into_iter()
            .filter_map(|k| self.problems.remove(&k))
            .collect()
    }

    /// Drops every open problem (no push) and returns them.
    pub fn clear(&mut self) -> Vec<Problem> {
        std::mem::take(&mut self.problems).into_values().collect()
    }

    /// The entity whose open problem was announced by `alert_id`.
    pub fn entity_for_alert(&self, alert_id: &str) -> Option<&str> {
        self.problems
            .values()
            .find(|p| p.alert_id.as_deref() == Some(alert_id) || p.alert_key() == alert_id)
            .map(|p| p.entity_id.as_str())
    }
}

/// Positive evidence that `p` is over. A check that cannot run any more is not evidence.
fn recovered(p: &Problem, r: &Reading, ctx: &Ctx) -> bool {
    match p.check {
        Check::Stuck => r.last_change.is_some_and(|t| t > p.opened_at),
        Check::Silent if ctx.silent_paused => false,
        Check::Silent if p.opened_offline() => r.unavailable_since.is_none(),
        Check::Silent => r.last_report.is_some_and(|t| t > p.opened_at),
        Check::Impossible => {
            match (
                physical_range(r.device_class.as_deref(), r.unit.as_deref()),
                r.value,
            ) {
                (Some((lo, hi)), Some(v)) => (lo..=hi).contains(&v),
                _ => false,
            }
        }
    }
}
