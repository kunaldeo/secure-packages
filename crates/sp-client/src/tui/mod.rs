mod app;
mod event;
mod ui;
mod views;

use std::io;
use std::process::ExitCode;
use std::time::Duration;

use crossterm::{
    cursor::{Hide, Show},
    event::{KeyCode, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use tokio::sync::mpsc;

use crate::api::SpClient;
use crate::commands::check::compute_exit_code;
use crate::resolver::ResolvedPackage;

use app::{App, View};
use event::{AppEvent, spawn_event_reader};

pub async fn run_tui(
    packages: Vec<ResolvedPackage>,
    server_url: &str,
    interval: Duration,
    fail_on_review: bool,
    requirements_file: String,
) -> ExitCode {
    // Install panic hook to restore terminal
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen, Show);
        original_hook(info);
    }));

    // Setup terminal
    enable_raw_mode().expect("failed to enable raw mode");
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, Hide).expect("failed to enter alternate screen");
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).expect("failed to create terminal");

    let mut app = App::new(fail_on_review, requirements_file, server_url.to_string());

    // Channel for poll results and detail fetches
    let (poll_tx, mut poll_rx) = mpsc::unbounded_channel::<AppEvent>();

    // Spawn poll task
    let poll_client = SpClient::new(server_url);
    let poll_interval = interval;
    tokio::spawn(async move {
        loop {
            let result = poll_client.check_packages(&packages).await;
            let event = AppEvent::PollResult(result.map_err(|e| e.to_string()));
            if poll_tx.send(event).is_err() {
                break;
            }
            tokio::time::sleep(poll_interval).await;
        }
    });

    // Spawn crossterm event reader
    let mut event_rx = spawn_event_reader(Duration::from_millis(80));

    // Sender for detail fetch tasks (cloned per fetch)
    let detail_base_tx = {
        let (tx, rx) = mpsc::unbounded_channel::<AppEvent>();
        // Merge detail events into poll_rx by forwarding
        // Actually, let's use a separate approach: store a second sender
        // that we check in the loop.
        // Simpler: just use one more channel.
        (tx, rx)
    };
    let (detail_tx, mut detail_rx) = detail_base_tx;

    // Main event loop
    loop {
        terminal.draw(|f| ui::draw(f, &app)).expect("draw failed");

        // Check for poll results (non-blocking)
        while let Ok(event) = poll_rx.try_recv() {
            match event {
                AppEvent::PollResult(Ok(statuses)) => {
                    app.error_message = None;
                    app.update_packages(statuses);
                }
                AppEvent::PollResult(Err(e)) => {
                    app.error_message = Some(e);
                }
                _ => {}
            }
        }

        // Check for detail results (non-blocking)
        while let Ok(event) = detail_rx.try_recv() {
            match event {
                AppEvent::DetailResult(Ok(details)) => {
                    app.detail_loading = false;
                    let key = (details.package.clone(), details.version.clone());
                    app.detail_cache.insert(key, details);
                }
                AppEvent::DetailResult(Err(e)) => {
                    app.detail_loading = false;
                    app.error_message = Some(format!("Failed to load details: {e}"));
                }
                _ => {}
            }
        }

        // Check for keyboard/tick events (blocking with timeout via recv)
        if let Some(event) = event_rx.recv().await {
            match event {
                AppEvent::Key(key) => {
                    handle_key(&mut app, key, &detail_tx);
                }
                AppEvent::Tick => {
                    app.tick += 1;
                }
                _ => {}
            }
        }

        if app.should_quit {
            break;
        }
    }

    // Restore terminal fully: raw mode off, leave alternate screen, show cursor
    let _ = terminal.clear();
    disable_raw_mode().expect("failed to disable raw mode");
    execute!(terminal.backend_mut(), LeaveAlternateScreen, Show)
        .expect("failed to leave alternate screen");

    // Print a brief summary to the restored terminal so the user sees final state
    let exit = compute_exit_code(&app.packages, app.fail_on_review);
    if !app.packages.is_empty() {
        let approved = app
            .packages
            .iter()
            .filter(|p| p.status == "approved")
            .count();
        let rejected = app
            .packages
            .iter()
            .filter(|p| p.status == "rejected" || p.status == "failed")
            .count();
        let pending = app
            .packages
            .iter()
            .filter(|p| p.status == "pending" || p.status == "analyzing")
            .count();
        let review = app
            .packages
            .iter()
            .filter(|p| p.status == "needs_review")
            .count();
        let total = app.packages.len();
        eprint!(
            "sp-client: {total} packages checked — \x1b[32m{approved} approved\x1b[0m, \x1b[31m{rejected} rejected\x1b[0m"
        );
        if pending > 0 {
            eprint!(", \x1b[33m{pending} pending\x1b[0m");
        }
        if review > 0 {
            eprint!(", \x1b[33m{review} needs review\x1b[0m");
        }
        eprintln!("  ({:.1}s)", app.elapsed_secs());
    }

    ExitCode::from(exit)
}

fn handle_key(
    app: &mut App,
    key: crossterm::event::KeyEvent,
    detail_tx: &mpsc::UnboundedSender<AppEvent>,
) {
    // Ctrl+C always quits
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        app.should_quit = true;
        return;
    }

    // Filter input mode
    if app.filter.active {
        match key.code {
            KeyCode::Esc => {
                app.filter.active = false;
                app.filter.query.clear();
                app.selected = 0;
            }
            KeyCode::Enter => {
                app.filter.active = false;
            }
            KeyCode::Backspace => {
                app.filter.query.pop();
                app.selected = 0;
            }
            KeyCode::Char(c) => {
                app.filter.query.push(c);
                app.selected = 0;
            }
            _ => {}
        }
        return;
    }

    match &app.view {
        View::PackageList => match key.code {
            KeyCode::Char('q') => app.should_quit = true,
            KeyCode::Char('j') | KeyCode::Down => app.select_next(),
            KeyCode::Char('k') | KeyCode::Up => app.select_prev(),
            KeyCode::Char('g') => app.select_first(),
            KeyCode::Char('G') => app.select_last(),
            KeyCode::Char('/') => {
                app.filter.active = true;
                app.filter.query.clear();
            }
            KeyCode::Esc => app.should_quit = true,
            KeyCode::Enter => open_detail(app, detail_tx),
            _ => {}
        },
        View::Detail { .. } => match key.code {
            KeyCode::Char('q') => app.should_quit = true,
            KeyCode::Esc => {
                app.view = View::PackageList;
                app.scroll_offset = 0;
            }
            KeyCode::Char('j') | KeyCode::Down => app.scroll_down(),
            KeyCode::Char('k') | KeyCode::Up => app.scroll_up(),
            KeyCode::Char('g') => app.scroll_top(),
            KeyCode::Char('G') => {
                app.scroll_offset = 500;
            }
            _ => {}
        },
    }
}

fn open_detail(app: &mut App, detail_tx: &mpsc::UnboundedSender<AppEvent>) {
    let Some(idx) = app.selected_package_index() else {
        return;
    };
    let p = &app.packages[idx];
    let key = (p.name.clone(), p.version.clone());

    app.scroll_offset = 0;
    app.view = View::Detail { index: idx };

    // Fetch details if not cached
    if !app.detail_cache.contains_key(&key) {
        app.detail_loading = true;
        let tx = detail_tx.clone();
        let server_url = app.server_url.clone();
        let name = key.0;
        let version = key.1;
        tokio::spawn(async move {
            let client = SpClient::new(&server_url);
            let result = client
                .get_analysis_details(&name, &version)
                .await
                .map_err(|e| e.to_string());
            let _ = tx.send(AppEvent::DetailResult(result));
        });
    }
}
