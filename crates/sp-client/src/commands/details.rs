use std::process::ExitCode;

use crate::api::SpClient;

pub struct DetailsArgs {
    pub package: String,
    pub version: String,
    pub server: String,
    pub json: bool,
}

pub async fn run(args: DetailsArgs) -> ExitCode {
    let client = SpClient::new(&args.server);

    let details = match client
        .get_analysis_details(&args.package, &args.version)
        .await
    {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Error: {e}");
            return ExitCode::FAILURE;
        }
    };

    if args.json {
        println!("{}", serde_json::to_string_pretty(&details).unwrap());
        return ExitCode::SUCCESS;
    }

    // Header
    println!(
        "Package: {} {} ({})",
        details.package, details.version, details.ecosystem
    );
    println!("Status:  {}", color_status(&details.status));

    match &details.analysis {
        None => {
            println!("Analysis: not yet available");
        }
        Some(a) => {
            println!(
                "Risk:    {}/1.0",
                a.risk_score
                    .map(|r| format!("{:.2}", r))
                    .unwrap_or_else(|| "-".to_string())
            );
            println!("Type:    {}", a.analysis_type);

            if let Some(model) = &a.model_used {
                print!("Model:   {model}");
                if let (Some(p), Some(c)) = (a.prompt_tokens, a.completion_tokens) {
                    print!(" ({p} prompt + {c} completion tokens)");
                }
                println!();
            }

            println!("Date:    {}", a.analyzed_at);
            println!();

            if let Some(reasoning) = &a.reasoning {
                println!("Reasoning:");
                println!("  {reasoning}");
                println!();
            }

            // Print findings from llm_result
            if let Some(flags) = a
                .llm_result
                .as_ref()
                .and_then(|llm| llm.get("flags"))
                .and_then(|f| f.as_array())
                .filter(|flags| !flags.is_empty())
            {
                println!("Findings ({}):", flags.len());
                for flag in flags {
                    let severity = flag
                        .get("severity")
                        .and_then(|s| s.as_str())
                        .unwrap_or("info");
                    let file = flag
                        .get("file_path")
                        .and_then(|s| s.as_str())
                        .unwrap_or("?");
                    let line = flag
                        .get("line_range")
                        .and_then(|s| s.as_str())
                        .unwrap_or("");
                    let desc = flag
                        .get("description")
                        .and_then(|s| s.as_str())
                        .unwrap_or("");
                    let confidence = flag
                        .get("confidence")
                        .and_then(|c| c.as_f64())
                        .map(|c| format!(" (confidence: {c:.1})"))
                        .unwrap_or_default();

                    let sev_colored = color_severity(severity);
                    let location = if line.is_empty() {
                        file.to_string()
                    } else {
                        format!("{file}:{line}")
                    };

                    println!("  [{sev_colored}] {location}{confidence}");
                    println!("    {desc}");
                    println!();
                }
            }
        }
    }

    ExitCode::SUCCESS
}

fn color_status(status: &str) -> String {
    match status {
        "approved" => format!("\x1b[32m{status}\x1b[0m"),
        "rejected" | "failed" => format!("\x1b[31m{status}\x1b[0m"),
        "pending" | "analyzing" | "needs_review" => format!("\x1b[33m{status}\x1b[0m"),
        _ => status.to_string(),
    }
}

fn color_severity(severity: &str) -> String {
    match severity {
        "critical" => format!("\x1b[31;1m{severity}\x1b[0m"),
        "high" => format!("\x1b[31m{severity}\x1b[0m"),
        "medium" => format!("\x1b[33m{severity}\x1b[0m"),
        "low" => format!("\x1b[36m{severity}\x1b[0m"),
        _ => format!("\x1b[37m{severity}\x1b[0m"),
    }
}
