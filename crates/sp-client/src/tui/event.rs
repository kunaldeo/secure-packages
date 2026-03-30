use std::time::Duration;

use crossterm::event::{self, Event, KeyEvent};
use tokio::sync::mpsc;

use crate::api::{AnalysisDetails, PackageStatus};

pub enum AppEvent {
    Key(KeyEvent),
    Tick,
    PollResult(Result<Vec<PackageStatus>, String>),
    DetailResult(Result<AnalysisDetails, String>),
}

/// Spawns a dedicated OS thread to read crossterm events and forward them
/// through a channel. Returns the receiver.
pub fn spawn_event_reader(tick_rate: Duration) -> mpsc::UnboundedReceiver<AppEvent> {
    let (tx, rx) = mpsc::unbounded_channel();

    std::thread::spawn(move || {
        loop {
            if event::poll(tick_rate).unwrap_or(false) {
                if let Ok(Event::Key(key)) = event::read() {
                    if tx.send(AppEvent::Key(key)).is_err() {
                        break;
                    }
                }
            } else if tx.send(AppEvent::Tick).is_err() {
                break;
            }
        }
    });

    rx
}
