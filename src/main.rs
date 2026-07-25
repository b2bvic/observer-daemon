mod config;
mod corrections;
mod rubric;
mod session;
mod validator;
mod validators;
mod watcher;

use rubric::{ClassificationMetadata, ContentClass};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

fn main() {
    env_logger::init();
    let args: Vec<String> = std::env::args().collect();
    let config_path = args
        .windows(2)
        .find(|window| window[0] == "--config")
        .map(|window| PathBuf::from(&window[1]))
        .unwrap_or_else(|| {
            let user_root = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            PathBuf::from(user_root).join(".observer").join("spec.toml")
        });
    let spec = load_or_exit(&config_path);

    if args.iter().any(|arg| arg == "--validate") {
        run_validate(&args, &spec);
    } else if args.iter().any(|arg| arg == "--daemon") {
        run_daemon(spec, config_path);
    } else {
        print_info(&spec, &config_path);
    }
}

fn load_or_exit(path: &Path) -> config::Spec {
    match config::load_spec(path) {
        Ok(spec) => spec,
        Err(error) => {
            eprintln!("Fatal: {error}");
            std::process::exit(1);
        }
    }
}

fn run_validate(args: &[String], spec: &config::Spec) {
    let validate_index = args
        .iter()
        .position(|argument| argument == "--validate")
        .expect("validate flag");
    let mut prompt = String::new();
    let mut metadata = ClassificationMetadata::default();
    let mut text_parts = Vec::new();
    let mut index = validate_index + 1;

    while index < args.len() {
        match args[index].as_str() {
            "--prompt" if index + 1 < args.len() => {
                prompt = args[index + 1].clone();
                index += 2;
            }
            "--content-class" if index + 1 < args.len() => {
                metadata.content_class = match args[index + 1].parse::<ContentClass>() {
                    Ok(content_class) => Some(content_class),
                    Err(error) => {
                        eprintln!("Fatal: {error}");
                        std::process::exit(2);
                    }
                };
                index += 2;
            }
            "--label" if index + 1 < args.len() => {
                metadata.labels.push(args[index + 1].clone());
                index += 2;
            }
            "--config" if index + 1 < args.len() => {
                index += 2;
            }
            _ => {
                text_parts.push(args[index].clone());
                index += 1;
            }
        }
    }

    let text = if text_parts.is_empty() {
        use std::io::Read;
        let mut buffer = String::new();
        std::io::stdin()
            .read_to_string(&mut buffer)
            .expect("read validation input");
        buffer
    } else {
        text_parts.join(" ")
    };

    let promoted_rules =
        corrections::load_patterns(&spec.daemon.expanded_patterns_path()).promoted_rules;
    let result = validator::validate_response_with_metadata(
        &text,
        &prompt,
        &metadata,
        &[],
        &promoted_rules,
        spec,
    );
    println!("Class: {}", result.content_class);
    println!("Score: {}/100", result.score);
    if result.issues.is_empty() {
        println!("No violations.");
    } else {
        for issue in &result.issues {
            println!("  {} (-{})", issue.display(), issue.deduction);
        }
    }
    if let Some(context) = result.correction_context(&spec.corrections) {
        println!("\nCorrection context:\n{context}");
    }
}

fn print_info(spec: &config::Spec, config_path: &Path) {
    println!("observer-daemon v{}", env!("CARGO_PKG_VERSION"));
    println!("Config: {}", config_path.display());
    println!(
        "Rubrics: legal, career_application, content_copy, technical_build, conversational, generic"
    );
    println!("Validators: 16");
    println!("Passing score: {}", spec.scoring.passing_score);
    println!("Watch paths: {}", spec.daemon.watch_paths.join(", "));
    println!("\nUse --daemon to watch files, or --validate <text> to score once.");
}

fn modified_time(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).ok()?.modified().ok()
}

fn run_daemon(mut spec: config::Spec, config_path: PathBuf) {
    let mut validation_log = spec.daemon.expanded_validation_log();
    let mut corrections_path = spec.daemon.expanded_corrections_path();
    let mut patterns_path = spec.daemon.expanded_patterns_path();
    let mut promoted_rules = corrections::load_patterns(&patterns_path).promoted_rules;
    promote_existing(
        &spec,
        &corrections_path,
        &patterns_path,
        &mut promoted_rules,
    );

    let mut file_watcher = match watcher::FileWatcher::new(
        &spec.daemon.expanded_watch_paths(),
        spec.daemon.watch_debounce_ms,
    ) {
        Ok(watcher) => watcher,
        Err(error) => {
            eprintln!("Fatal: {error}");
            std::process::exit(1);
        }
    };
    let mut last_config_mtime = modified_time(&config_path);
    let reload = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let shutdown = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let _ = signal_hook::flag::register(signal_hook::consts::SIGHUP, Arc::clone(&reload));
    let _ = signal_hook::flag::register(signal_hook::consts::SIGTERM, Arc::clone(&shutdown));
    let _ = signal_hook::flag::register(signal_hook::consts::SIGINT, Arc::clone(&shutdown));
    let mut prior_responses = Vec::<String>::new();

    log::info!(
        "Daemon watching {} configured paths",
        spec.daemon.watch_paths.len()
    );
    while !shutdown.load(std::sync::atomic::Ordering::Relaxed) {
        let current_mtime = modified_time(&config_path);
        let changed = current_mtime != last_config_mtime;
        if reload.swap(false, std::sync::atomic::Ordering::Relaxed) || changed {
            match config::load_spec(&config_path) {
                Ok(new_spec) => {
                    let previous_offsets = file_watcher.offsets.clone();
                    match watcher::FileWatcher::new(
                        &new_spec.daemon.expanded_watch_paths(),
                        new_spec.daemon.watch_debounce_ms,
                    ) {
                        Ok(mut new_watcher) => {
                            new_watcher.restore_offsets(&previous_offsets);
                            spec = new_spec;
                            file_watcher = new_watcher;
                            validation_log = spec.daemon.expanded_validation_log();
                            corrections_path = spec.daemon.expanded_corrections_path();
                            patterns_path = spec.daemon.expanded_patterns_path();
                            promoted_rules =
                                corrections::load_patterns(&patterns_path).promoted_rules;
                            last_config_mtime = current_mtime;
                            log::info!("Configuration reloaded");
                        }
                        Err(error) => log::error!("Watcher reload failed: {error}"),
                    }
                }
                Err(error) => log::error!("Configuration reload failed: {error}"),
            }
        }

        for path in file_watcher.poll() {
            let extension = path
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or("");
            if extension == "jsonl" {
                process_jsonl(
                    &path,
                    &mut file_watcher,
                    &mut prior_responses,
                    &mut promoted_rules,
                    &spec,
                    &validation_log,
                    &corrections_path,
                    &patterns_path,
                );
            } else if extension == "md" {
                process_markdown(
                    &path,
                    &mut file_watcher,
                    &mut prior_responses,
                    &mut promoted_rules,
                    &spec,
                    &validation_log,
                    &corrections_path,
                    &patterns_path,
                );
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    log::info!("Daemon stopped");
}

#[allow(clippy::too_many_arguments)]
fn process_jsonl(
    path: &Path,
    file_watcher: &mut watcher::FileWatcher,
    prior_responses: &mut Vec<String>,
    promoted_rules: &mut Vec<corrections::PromotedRule>,
    spec: &config::Spec,
    validation_log: &Path,
    corrections_path: &Path,
    patterns_path: &Path,
) {
    let offset = file_watcher.get_offset(path);
    let Ok((lines, new_offset)) = session::read_new_lines(path, offset) else {
        return;
    };
    file_watcher.set_offset(path.to_path_buf(), new_offset);
    let is_responses_format = path.to_string_lossy().contains("/.codex/sessions/");
    let session_id = session::codex_session_id_from_path(path);
    let messages = if is_responses_format {
        session::extract_codex_messages(&lines, &session_id)
    } else {
        session::extract_assistant_messages(&lines)
    };
    let source = if is_responses_format {
        "responses-jsonl"
    } else {
        "message-jsonl"
    };

    for message in messages {
        let result = validator::validate_response_with_metadata(
            &message.text,
            &message.prompt,
            &message.metadata,
            prior_responses,
            promoted_rules,
            spec,
        );
        record_result(
            &message.text,
            &message.session_id,
            source,
            &result,
            promoted_rules,
            spec,
            validation_log,
            corrections_path,
            patterns_path,
        );
        retain_recent(prior_responses, message.text);
    }
}

#[allow(clippy::too_many_arguments)]
fn process_markdown(
    path: &Path,
    file_watcher: &mut watcher::FileWatcher,
    prior_responses: &mut Vec<String>,
    promoted_rules: &mut Vec<corrections::PromotedRule>,
    spec: &config::Spec,
    validation_log: &Path,
    corrections_path: &Path,
    patterns_path: &Path,
) {
    let offset = file_watcher.get_offset(path);
    let Ok(content) = std::fs::read_to_string(path) else {
        return;
    };
    let exchanges = session::extract_markdown_exchanges(
        &content,
        usize::try_from(offset).unwrap_or(usize::MAX),
    );
    file_watcher.set_offset(path.to_path_buf(), content.len() as u64);

    for exchange in exchanges {
        let result = validator::validate_response(
            &exchange.response,
            &exchange.prompt,
            prior_responses,
            promoted_rules,
            spec,
        );
        record_result(
            &exchange.response,
            "markdown",
            "markdown",
            &result,
            promoted_rules,
            spec,
            validation_log,
            corrections_path,
            patterns_path,
        );
        retain_recent(prior_responses, exchange.response);
    }
}

fn retain_recent(prior_responses: &mut Vec<String>, response: String) {
    prior_responses.push(response);
    if prior_responses.len() > 20 {
        prior_responses.drain(0..10);
    }
}

#[allow(clippy::too_many_arguments)]
fn record_result(
    response: &str,
    session_id: &str,
    source: &str,
    result: &validator::ValidationResult,
    promoted_rules: &mut Vec<corrections::PromotedRule>,
    spec: &config::Spec,
    validation_log: &Path,
    corrections_path: &Path,
    patterns_path: &Path,
) {
    if let Ok(json) = serde_json::to_string(&result.to_json(session_id, source)) {
        let _ = append_line(validation_log, &json);
    }
    if result.score >= spec.scoring.passing_score {
        return;
    }

    for issue in &result.issues {
        let correction = corrections::CorrectionEntry {
            ts: chrono::Local::now().to_rfc3339(),
            violation_type: issue.category.clone(),
            pattern: issue.detail.clone(),
            session: session_id.to_string(),
            score: result.score,
            source: source.to_string(),
            context: response.chars().take(200).collect(),
            content_class: result.content_class,
        };
        let _ = corrections::write_correction(corrections_path, &correction);
    }
    if !spec.corrections.auto_promote {
        return;
    }
    let keys: HashSet<(ContentClass, String, String)> = result
        .issues
        .iter()
        .map(|issue| {
            (
                result.content_class,
                issue.category.clone(),
                issue.detail.clone(),
            )
        })
        .collect();
    if corrections::should_promote_any(
        corrections_path,
        &keys,
        spec.corrections.promotion_threshold,
        spec.corrections.aggregation_window_days,
    ) {
        let new_rules = corrections::aggregate_and_promote(
            corrections_path,
            patterns_path,
            spec.corrections.promotion_threshold,
            spec.corrections.aggregation_window_days,
        );
        if !new_rules.is_empty() {
            *promoted_rules = corrections::load_patterns(patterns_path).promoted_rules;
        }
    }
}

fn promote_existing(
    spec: &config::Spec,
    corrections_path: &Path,
    patterns_path: &Path,
    promoted_rules: &mut Vec<corrections::PromotedRule>,
) {
    if !spec.corrections.auto_promote {
        return;
    }
    let new_rules = corrections::aggregate_and_promote(
        corrections_path,
        patterns_path,
        spec.corrections.promotion_threshold,
        spec.corrections.aggregation_window_days,
    );
    if !new_rules.is_empty() {
        *promoted_rules = corrections::load_patterns(patterns_path).promoted_rules;
    }
}

fn append_line(path: &Path, line: &str) -> std::io::Result<()> {
    use std::io::Write;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(file, "{line}")
}
