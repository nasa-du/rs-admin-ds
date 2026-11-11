//! Argon2id calibration for the deployment host (spec §8.1, §12.6).
//!
//! Measures verification latency for the policy parameter set
//! `m = 64 MiB, t = 3, p = 4` and, when its p95 exceeds the 300 ms target, the
//! documented fallback sequence. Run it through `scripts/bench-argon2.sh`: the
//! wrapper pins the CPU budget that the API container actually receives and
//! records the report in `docs/acceptance/`.
//!
//! The unit of work is one `verify_password` call on a freshly parsed PHC
//! string, which is what the login path performs per request.

use argon2::password_hash::phc::PasswordHash;
use argon2::password_hash::{PasswordHasher as _, PasswordVerifier as _};
use argon2::{Algorithm, Argon2, Params, Version};
use serde::Serialize;
use std::time::{Duration, Instant};

/// Target upper bound for a single verification (spec §8.1).
pub const TARGET_P95_MS: f64 = 300.0;
/// Absolute upper bound; above this the parameter set is unusable.
pub const HARD_MAX_P95_MS: f64 = 1_000.0;

/// Output length and salt length fixed by the policy (spec §8.1).
const OUTPUT_BYTES: usize = 32;

/// Password used for calibration. It never belongs to any account.
const BENCH_PASSWORD: &str = "rs-admin-argon2-calibration-password";

/// One Argon2id parameter set.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Candidate {
    /// Memory cost in KiB.
    pub m_kib: u32,
    /// Time cost (iterations).
    pub t: u32,
    /// Parallelism (lanes).
    pub p: u32,
}

impl Candidate {
    pub const fn new(m_kib: u32, t: u32, p: u32) -> Self {
        Self { m_kib, t, p }
    }

    fn label(self) -> String {
        format!("m={} MiB,t={},p={}", self.m_kib / 1024, self.t, self.p)
    }
}

/// Policy set first, then the exact fallback order from spec §8.1: lower
/// parallelism, then half the memory, then the OWASP floor.
pub const CANDIDATES: [Candidate; 4] = [
    Candidate::new(65_536, 3, 4),
    Candidate::new(65_536, 3, 1),
    Candidate::new(32_768, 3, 1),
    Candidate::new(19_456, 2, 1),
];

/// Measured latency distribution for one candidate.
#[derive(Clone, Debug, Serialize)]
pub struct CandidateReport {
    pub m_kib: u32,
    pub t: u32,
    pub p: u32,
    pub iterations: usize,
    pub min_ms: f64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub max_ms: f64,
    pub mean_ms: f64,
    pub within_target: bool,
    pub within_hard_limit: bool,
}

#[derive(Serialize)]
struct Environment {
    os: &'static str,
    arch: &'static str,
    logical_cpus: usize,
    cpu_affinity: String,
}

#[derive(Serialize)]
struct Report {
    environment: Environment,
    iterations: usize,
    target_p95_ms: f64,
    hard_max_p95_ms: f64,
    candidates: Vec<CandidateReport>,
    recommended: Option<Candidate>,
    within_target: bool,
    within_hard_limit: bool,
}

fn main() {
    let options = match Options::parse(std::env::args().skip(1)) {
        Ok(OptionsOutcome::Run(options)) => options,
        Ok(OptionsOutcome::Help) => {
            print!("{}", usage());
            return;
        }
        Err(error) => {
            eprintln!("argon2-bench: {error}\n\n{}", usage());
            std::process::exit(2);
        }
    };

    let report = match run(&options) {
        Ok(report) => report,
        Err(error) => {
            eprintln!("argon2-bench: {error}");
            std::process::exit(1);
        }
    };

    if options.json {
        match serde_json::to_string_pretty(&report) {
            Ok(json) => println!("{json}"),
            Err(error) => {
                eprintln!("argon2-bench: serializing the report failed: {error}");
                std::process::exit(1);
            }
        }
    } else {
        print_human(&report);
    }

    // Exit codes let the wrapper distinguish "on target" from "fallback
    // needed" and "unusable hardware" without parsing the output.
    if !report.within_hard_limit {
        std::process::exit(4);
    }
    if !report.within_target {
        std::process::exit(3);
    }
}

/// Measure candidates in order and stop at the first one whose p95 fits the
/// target.
fn run(options: &Options) -> Result<Report, String> {
    let sequence: Vec<Candidate> = if options.candidates.is_empty() {
        CANDIDATES.to_vec()
    } else {
        options.candidates.clone()
    };

    let mut candidates = Vec::new();
    for candidate in sequence {
        let measured = measure(candidate, options.iterations)?;
        let fits = measured.within_target;
        candidates.push(measured);
        if fits {
            break;
        }
    }

    let recommended = recommend(&candidates);
    let within_target = recommended.is_some();
    let within_hard_limit = candidates
        .iter()
        .any(|candidate| candidate.within_hard_limit);

    Ok(Report {
        environment: environment(),
        iterations: options.iterations,
        target_p95_ms: TARGET_P95_MS,
        hard_max_p95_ms: HARD_MAX_P95_MS,
        candidates,
        recommended,
        within_target,
        within_hard_limit,
    })
}

/// The first candidate whose p95 fits the target, in fallback order.
fn recommend(candidates: &[CandidateReport]) -> Option<Candidate> {
    candidates
        .iter()
        .find(|candidate| candidate.within_target)
        .map(|candidate| Candidate::new(candidate.m_kib, candidate.t, candidate.p))
}

fn measure(candidate: Candidate, iterations: usize) -> Result<CandidateReport, String> {
    let params = Params::new(
        candidate.m_kib,
        candidate.t,
        candidate.p,
        Some(OUTPUT_BYTES),
    )
    .map_err(|error| format!("invalid parameters {}: {error}", candidate.label()))?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let phc = argon2
        .hash_password(BENCH_PASSWORD.as_bytes())
        .map_err(|error| format!("hashing the calibration password failed: {error}"))?
        .to_string();

    // One warm-up verification: page in the memory and the thread pool
    // before recording.
    verify_once(&argon2, &phc)?;

    let mut samples = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let started = Instant::now();
        verify_once(&argon2, &phc)?;
        samples.push(started.elapsed());
    }
    samples.sort_unstable();

    let total: Duration = samples.iter().sum();
    let mean = total.as_secs_f64() * 1_000.0 / samples.len() as f64;
    let p95_ms = to_ms(percentile(&samples, 0.95));

    Ok(CandidateReport {
        m_kib: candidate.m_kib,
        t: candidate.t,
        p: candidate.p,
        iterations,
        min_ms: to_ms(samples[0]),
        p50_ms: to_ms(percentile(&samples, 0.50)),
        p95_ms,
        max_ms: to_ms(*samples.last().expect("at least one sample")),
        mean_ms: round(mean),
        within_target: p95_ms <= TARGET_P95_MS,
        within_hard_limit: p95_ms <= HARD_MAX_P95_MS,
    })
}

fn verify_once(argon2: &Argon2<'static>, phc: &str) -> Result<(), String> {
    let parsed =
        PasswordHash::new(phc).map_err(|error| format!("re-parsing the PHC failed: {error}"))?;
    argon2
        .verify_password(BENCH_PASSWORD.as_bytes(), &parsed)
        .map_err(|error| format!("verification failed: {error}"))
}

/// Nearest-rank percentile. `sorted` must be non-empty and ascending.
fn percentile(sorted: &[Duration], fraction: f64) -> Duration {
    let rank = (fraction * sorted.len() as f64).ceil() as usize;
    sorted[rank.saturating_sub(1).min(sorted.len() - 1)]
}

fn to_ms(duration: Duration) -> f64 {
    round(duration.as_secs_f64() * 1_000.0)
}

fn round(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

fn environment() -> Environment {
    Environment {
        os: std::env::consts::OS,
        arch: std::env::consts::ARCH,
        logical_cpus: std::thread::available_parallelism()
            .map(|count| count.get())
            .unwrap_or(1),
        cpu_affinity: cpu_affinity(),
    }
}

/// Parse `Cpus_allowed_list` from `/proc/self/status` so the record shows the
/// CPU budget the measurement actually received (the wrapper uses taskset).
fn cpu_affinity() -> String {
    let Ok(status) = std::fs::read_to_string("/proc/self/status") else {
        return "unknown".to_owned();
    };
    status
        .lines()
        .find_map(|line| line.strip_prefix("Cpus_allowed_list:"))
        .map(|value| value.trim().to_owned())
        .unwrap_or_else(|| "unknown".to_owned())
}

fn print_human(report: &Report) {
    println!(
        "Argon2id calibration: {} on {}/{} cores (affinity {}), {} iterations",
        report.environment.os,
        report.environment.logical_cpus,
        report.environment.arch,
        report.environment.cpu_affinity,
        report.iterations
    );
    println!();
    println!(
        "{:<24} {:>9} {:>9} {:>9} {:>9} {:>9}  target",
        "parameters", "min ms", "p50 ms", "p95 ms", "max ms", "mean ms"
    );
    for candidate in &report.candidates {
        let verdict = if candidate.within_target {
            "ok (<=300 ms)"
        } else if candidate.within_hard_limit {
            "above target (<=1 s)"
        } else {
            "unusable (>1 s)"
        };
        println!(
            "{:<24} {:>9.2} {:>9.2} {:>9.2} {:>9.2} {:>9.2}  {}",
            Candidate::new(candidate.m_kib, candidate.t, candidate.p).label(),
            candidate.min_ms,
            candidate.p50_ms,
            candidate.p95_ms,
            candidate.max_ms,
            candidate.mean_ms,
            verdict
        );
    }
    println!();
    match report.recommended {
        Some(candidate) => {
            println!("Recommended parameters: {}", candidate.label());
            if candidate != Candidate::new(65_536, 3, 4) {
                println!(
                    "The policy default exceeded the target; apply the fallback and update \
                     ARGON2_MEMORY_KIB/ARGON2_TIME_COST/ARGON2_PARALLELISM."
                );
            }
        }
        None if report.within_hard_limit => {
            println!(
                "No candidate reached the 300 ms target. The OWASP floor remains under 1 s; \
                 record a risk acceptance before deploying."
            );
        }
        None => {
            println!(
                "Every candidate exceeded 1 s. This hardware cannot run the policy; resize the \
                 host/container CPU budget before deploying."
            );
        }
    }
}

#[derive(Clone, Debug)]
struct Options {
    iterations: usize,
    candidates: Vec<Candidate>,
    json: bool,
}

enum OptionsOutcome {
    Run(Options),
    Help,
}

impl Options {
    fn parse(arguments: impl IntoIterator<Item = String>) -> Result<OptionsOutcome, String> {
        let mut options = Options {
            iterations: 30,
            candidates: Vec::new(),
            json: false,
        };
        let mut arguments = arguments.into_iter();

        while let Some(argument) = arguments.next() {
            match argument.as_str() {
                "--help" | "-h" => return Ok(OptionsOutcome::Help),
                "--json" => options.json = true,
                "--iterations" => {
                    let value = arguments
                        .next()
                        .ok_or_else(|| "--iterations requires a value".to_owned())?;
                    let iterations = value
                        .parse::<usize>()
                        .map_err(|_| format!("invalid iteration count: {value}"))?;
                    if iterations == 0 {
                        return Err("iteration count must be at least one".to_owned());
                    }
                    options.iterations = iterations;
                }
                "--candidate" => {
                    let value = arguments
                        .next()
                        .ok_or_else(|| "--candidate requires m,t,p".to_owned())?;
                    options.candidates.push(parse_candidate(&value)?);
                }
                other => return Err(format!("unknown argument: {other}")),
            }
        }

        Ok(OptionsOutcome::Run(options))
    }
}

fn parse_candidate(value: &str) -> Result<Candidate, String> {
    let parts: Vec<&str> = value.split(',').collect();
    if parts.len() != 3 {
        return Err(format!("invalid candidate '{value}': expected m_kib,t,p"));
    }
    let number = |part: &str, name: &str| {
        part.trim()
            .parse::<u32>()
            .map_err(|_| format!("invalid {name} in '{value}'"))
    };
    let candidate = Candidate::new(
        number(parts[0], "memory")?,
        number(parts[1], "time cost")?,
        number(parts[2], "parallelism")?,
    );
    Params::new(
        candidate.m_kib,
        candidate.t,
        candidate.p,
        Some(OUTPUT_BYTES),
    )
    .map_err(|error| format!("invalid candidate '{value}': {error}"))?;
    Ok(candidate)
}

fn usage() -> &'static str {
    "\
argon2-bench - calibrate Argon2id parameters on this host (spec §8.1)

Usage:
  argon2-bench [--iterations N] [--candidate m_kib,t,p]... [--json]

Without --candidate, measures the policy set (65536,3,4) and stops at the
first candidate in the documented fallback sequence whose p95 fits 300 ms.
Exit codes: 0 target met, 3 fallback below target needed, 4 unusable (>1 s).
"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn durations(values: &[u64]) -> Vec<Duration> {
        values
            .iter()
            .map(|value| Duration::from_millis(*value))
            .collect()
    }

    #[test]
    fn percentile_uses_nearest_rank() {
        let sorted = durations(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
        assert_eq!(percentile(&sorted, 0.50), Duration::from_millis(5));
        assert_eq!(percentile(&sorted, 0.95), Duration::from_millis(10));
        assert_eq!(percentile(&sorted, 0.99), Duration::from_millis(10));
        assert_eq!(percentile(&sorted, 0.0), Duration::from_millis(1));
    }

    #[test]
    fn fallback_sequence_matches_the_documented_order_and_floor() {
        assert_eq!(CANDIDATES[0], Candidate::new(65_536, 3, 4));
        assert_eq!(CANDIDATES[1], Candidate::new(65_536, 3, 1));
        assert_eq!(CANDIDATES[2], Candidate::new(32_768, 3, 1));
        assert_eq!(CANDIDATES[3], Candidate::new(19_456, 2, 1));
    }

    #[test]
    fn explicit_candidates_are_parsed_and_validated() {
        assert_eq!(
            parse_candidate("65536,3,4").expect("candidate"),
            Candidate::new(65_536, 3, 4)
        );
        assert!(parse_candidate("65536,3").is_err());
        assert!(parse_candidate("0,3,1").is_err());
        assert!(parse_candidate("65536,3,0").is_err());
    }

    #[test]
    fn recommendation_selects_the_first_candidate_within_target() {
        let candidates = [
            candidate_report(Candidate::new(65_536, 3, 4), 410.0),
            candidate_report(Candidate::new(65_536, 3, 1), 260.0),
            candidate_report(Candidate::new(32_768, 3, 1), 200.0),
        ];
        assert_eq!(
            recommend(&candidates),
            Some(Candidate::new(65_536, 3, 1)),
            "the first candidate in fallback order wins"
        );
        assert_eq!(recommend(&candidates[..1]), None, "no candidate fits");
    }

    fn candidate_report(candidate: Candidate, p95_ms: f64) -> CandidateReport {
        CandidateReport {
            m_kib: candidate.m_kib,
            t: candidate.t,
            p: candidate.p,
            iterations: 1,
            min_ms: p95_ms,
            p50_ms: p95_ms,
            p95_ms,
            max_ms: p95_ms,
            mean_ms: p95_ms,
            within_target: p95_ms <= TARGET_P95_MS,
            within_hard_limit: p95_ms <= HARD_MAX_P95_MS,
        }
    }
}
