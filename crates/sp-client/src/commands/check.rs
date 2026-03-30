use std::collections::HashMap;
use std::path::Path;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use crate::api::{PackageStatus, SpClient};
use crate::resolver::{self, Resolver};

pub struct CheckArgs {
    pub requirements: String,
    pub server: String,
    pub watch: bool,
    pub interval: u64,
    pub json: bool,
    pub fail_on_review: bool,
    pub resolver: Option<Resolver>,
    pub interactive: bool,
}

pub async fn run(args: CheckArgs) -> ExitCode {
    // Resolve dependencies
    eprintln!("Resolving dependencies...");
    let packages = match resolver::resolve(Path::new(&args.requirements), args.resolver).await {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Error resolving dependencies: {e}");
            return ExitCode::from(2);
        }
    };

    if packages.is_empty() {
        eprintln!("No packages to check.");
        return ExitCode::SUCCESS;
    }

    eprintln!("Resolved {} packages.", packages.len());

    // Interactive TUI mode
    if args.interactive {
        return crate::tui::run_tui(
            packages,
            &args.server,
            Duration::from_secs(args.interval),
            args.fail_on_review,
            args.requirements.clone(),
        )
        .await;
    }

    // Non-interactive mode (original behavior)
    eprintln!("Submitting for analysis...");
    if args.watch {
        eprintln!("Waiting for results. Ctrl+C to stop waiting (analysis continues on server).");
        eprintln!(
            "Use --no-wait to submit and exit immediately. Re-run the same command to check progress."
        );
    }

    let client = SpClient::new(&args.server);
    let start = Instant::now();
    let mut prev_statuses: HashMap<String, String> = HashMap::new();
    let mut first_poll = true;

    loop {
        let statuses = match client.check_packages(&packages).await {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Error communicating with server: {e}");
                return ExitCode::from(2);
            }
        };

        if args.json && !args.watch {
            // One-shot JSON mode
            println!("{}", serde_json::to_string_pretty(&statuses).unwrap());
            return ExitCode::from(compute_exit_code(&statuses, args.fail_on_review));
        }

        // Print status transitions
        if !args.json {
            for s in &statuses {
                let key = format!("{}=={}", s.name, s.version);
                let prev = prev_statuses.get(&key).map(|s| s.as_str()).unwrap_or("");
                if prev != s.status {
                    if !first_poll || s.status != "pending" {
                        print_status_change(s, &start);
                    }
                }
                prev_statuses.insert(key, s.status.clone());
            }

            if first_poll {
                let pending = statuses.iter().filter(|s| s.status == "pending").count();
                if pending > 0 {
                    eprintln!(
                        "[{:>6.1}s] {} packages queued for analysis",
                        start.elapsed().as_secs_f64(),
                        pending
                    );
                }
            }
        }

        first_poll = false;
        let exit = compute_exit_code(&statuses, args.fail_on_review);

        if !args.watch || exit != 2 {
            // All resolved — print final table
            if args.json {
                println!("{}", serde_json::to_string_pretty(&statuses).unwrap());
            } else {
                eprintln!();
                print_table(&statuses, start.elapsed());
            }
            return ExitCode::from(exit);
        }

        // Still pending — wait and poll again
        tokio::time::sleep(Duration::from_secs(args.interval)).await;
    }
}

fn print_status_change(s: &PackageStatus, start: &Instant) {
    let elapsed = start.elapsed().as_secs_f64();
    let icon = match s.status.as_str() {
        "approved" => "\x1b[32m+\x1b[0m",
        "rejected" | "failed" => "\x1b[31mx\x1b[0m",
        "analyzing" => "\x1b[33m~\x1b[0m",
        "needs_review" => "\x1b[33m?\x1b[0m",
        _ => " ",
    };

    let detail = match s.status.as_str() {
        "approved" => s
            .reasoning
            .as_deref()
            .map(|r| truncate(r, 60))
            .unwrap_or_default(),
        "rejected" => s
            .reasoning
            .as_deref()
            .map(|r| truncate(r, 60))
            .unwrap_or_else(|| "rejected".to_string()),
        "failed" => s
            .error
            .as_deref()
            .map(|e| truncate(e, 60))
            .unwrap_or_else(|| "analysis failed".to_string()),
        "analyzing" => "analyzing...".to_string(),
        "needs_review" => "flagged for review".to_string(),
        _ => String::new(),
    };

    let risk = s
        .risk_score
        .map(|r| format!(" (risk: {r:.2})"))
        .unwrap_or_default();

    eprintln!(
        "[{elapsed:>6.1}s] {icon} {name}=={version} {status}{risk} {detail}",
        name = s.name,
        version = s.version,
        status = s.status,
    );
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() > max {
        format!("{}...", &s[..max - 3])
    } else {
        s.to_string()
    }
}

fn print_table(statuses: &[PackageStatus], elapsed: Duration) {
    eprintln!(
        "{:<30} {:<12} {:<12} {:<8} Summary",
        "Package", "Version", "Status", "Risk",
    );
    eprintln!("{}", "-".repeat(90));

    for s in statuses {
        let status_colored = match s.status.as_str() {
            "approved" => format!("\x1b[32m{:<12}\x1b[0m", s.status),
            "rejected" | "failed" => format!("\x1b[31m{:<12}\x1b[0m", s.status),
            _ => format!("\x1b[33m{:<12}\x1b[0m", s.status),
        };

        let risk = s
            .risk_score
            .map(|r| format!("{:.2}", r))
            .unwrap_or_else(|| "-".to_string());

        let summary = if s.status == "failed" {
            truncate(s.error.as_deref().unwrap_or("unknown error"), 40)
        } else {
            truncate(s.reasoning.as_deref().unwrap_or("-"), 40)
        };

        eprintln!(
            "{:<30} {:<12} {} {:<8} {}",
            s.name, s.version, status_colored, risk, summary
        );
    }

    eprintln!("{}", "-".repeat(90));

    let approved = statuses.iter().filter(|s| s.status == "approved").count();
    let rejected = statuses
        .iter()
        .filter(|s| s.status == "rejected" || s.status == "failed")
        .count();
    let pending = statuses
        .iter()
        .filter(|s| s.status == "pending" || s.status == "analyzing")
        .count();
    let review = statuses
        .iter()
        .filter(|s| s.status == "needs_review")
        .count();

    eprint!(
        "\x1b[32m{approved} approved\x1b[0m, \x1b[31m{rejected} rejected\x1b[0m, \
         \x1b[33m{pending} pending\x1b[0m"
    );
    if review > 0 {
        eprint!(", \x1b[33m{review} needs review\x1b[0m");
    }
    eprintln!("  ({:.1}s)", elapsed.as_secs_f64());
}

/// Compute exit code:
/// 0 = all approved
/// 1 = any rejected or failed (or needs_review if --fail-on-review)
/// 2 = still pending/analyzing
pub fn compute_exit_code(statuses: &[PackageStatus], fail_on_review: bool) -> u8 {
    let has_pending = statuses
        .iter()
        .any(|s| s.status == "pending" || s.status == "analyzing");
    let has_rejected = statuses
        .iter()
        .any(|s| s.status == "rejected" || s.status == "failed");
    let has_review = statuses.iter().any(|s| s.status == "needs_review");

    if has_pending {
        return 2;
    }
    if has_rejected {
        return 1;
    }
    if fail_on_review && has_review {
        return 1;
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_status(name: &str, status: &str) -> PackageStatus {
        PackageStatus {
            name: name.to_string(),
            version: "1.0.0".to_string(),
            ecosystem: "pypi".to_string(),
            version_id: None,
            status: status.to_string(),
            risk_score: None,
            verdict: None,
            reasoning: None,
            error: None,
        }
    }

    #[test]
    fn test_exit_code_all_approved() {
        let statuses = vec![make_status("a", "approved"), make_status("b", "approved")];
        assert_eq!(compute_exit_code(&statuses, false), 0);
    }

    #[test]
    fn test_exit_code_has_rejected() {
        let statuses = vec![make_status("a", "approved"), make_status("b", "rejected")];
        assert_eq!(compute_exit_code(&statuses, false), 1);
    }

    #[test]
    fn test_exit_code_has_failed() {
        let statuses = vec![make_status("a", "approved"), make_status("b", "failed")];
        assert_eq!(compute_exit_code(&statuses, false), 1);
    }

    #[test]
    fn test_exit_code_has_pending() {
        let statuses = vec![make_status("a", "approved"), make_status("b", "pending")];
        assert_eq!(compute_exit_code(&statuses, false), 2);
    }

    #[test]
    fn test_exit_code_pending_takes_priority() {
        let statuses = vec![make_status("a", "rejected"), make_status("b", "pending")];
        assert_eq!(compute_exit_code(&statuses, false), 2);
    }

    #[test]
    fn test_exit_code_needs_review_normal() {
        let statuses = vec![
            make_status("a", "approved"),
            make_status("b", "needs_review"),
        ];
        assert_eq!(compute_exit_code(&statuses, false), 0);
    }

    #[test]
    fn test_exit_code_needs_review_strict() {
        let statuses = vec![
            make_status("a", "approved"),
            make_status("b", "needs_review"),
        ];
        assert_eq!(compute_exit_code(&statuses, true), 1);
    }
}
