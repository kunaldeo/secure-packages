use std::collections::HashMap;
use std::time::Instant;

use crate::api::{AnalysisDetails, PackageStatus};

const SPINNER_FRAMES: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

#[derive(Debug, Clone, PartialEq)]
pub enum View {
    PackageList,
    Detail { index: usize },
}

pub struct FilterState {
    pub active: bool,
    pub query: String,
}

pub struct App {
    pub packages: Vec<PackageStatus>,
    pub selected: usize,
    pub view: View,
    pub filter: FilterState,
    pub detail_cache: HashMap<(String, String), AnalysisDetails>,
    pub detail_loading: bool,
    pub start_time: Instant,
    pub all_resolved: bool,
    pub error_message: Option<String>,
    pub should_quit: bool,
    pub fail_on_review: bool,
    pub tick: usize,
    pub scroll_offset: u16,
    pub requirements_file: String,
    pub server_url: String,
}

impl App {
    pub fn new(fail_on_review: bool, requirements_file: String, server_url: String) -> Self {
        Self {
            packages: Vec::new(),
            selected: 0,
            view: View::PackageList,
            filter: FilterState {
                active: false,
                query: String::new(),
            },
            detail_cache: HashMap::new(),
            detail_loading: false,
            start_time: Instant::now(),
            all_resolved: false,
            error_message: None,
            should_quit: false,
            fail_on_review,
            tick: 0,
            scroll_offset: 0,
            requirements_file,
            server_url,
        }
    }

    pub fn spinner_char(&self) -> char {
        SPINNER_FRAMES[self.tick % SPINNER_FRAMES.len()]
    }

    pub fn filtered_indices(&self) -> Vec<usize> {
        if self.filter.query.is_empty() {
            return (0..self.packages.len()).collect();
        }
        let q = self.filter.query.to_lowercase();
        self.packages
            .iter()
            .enumerate()
            .filter(|(_, p)| p.name.to_lowercase().contains(&q) || p.version.contains(&q))
            .map(|(i, _)| i)
            .collect()
    }

    pub fn select_next(&mut self) {
        let indices = self.filtered_indices();
        if indices.is_empty() {
            return;
        }
        if self.selected + 1 < indices.len() {
            self.selected += 1;
        }
    }

    pub fn select_prev(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
        }
    }

    pub fn select_first(&mut self) {
        self.selected = 0;
    }

    pub fn select_last(&mut self) {
        let indices = self.filtered_indices();
        if !indices.is_empty() {
            self.selected = indices.len() - 1;
        }
    }

    /// Get the actual package index for the currently selected filtered row.
    pub fn selected_package_index(&self) -> Option<usize> {
        let indices = self.filtered_indices();
        indices.get(self.selected).copied()
    }

    pub fn update_packages(&mut self, new_packages: Vec<PackageStatus>) {
        self.packages = new_packages;
        self.all_resolved = self
            .packages
            .iter()
            .all(|p| !matches!(p.status.as_str(), "pending" | "analyzing"));

        // Clamp selection
        let count = self.filtered_indices().len();
        if count > 0 && self.selected >= count {
            self.selected = count - 1;
        }
    }

    pub fn scroll_down(&mut self) {
        self.scroll_offset = self.scroll_offset.saturating_add(1);
    }

    pub fn scroll_up(&mut self) {
        self.scroll_offset = self.scroll_offset.saturating_sub(1);
    }

    pub fn scroll_top(&mut self) {
        self.scroll_offset = 0;
    }

    pub fn elapsed_secs(&self) -> f64 {
        self.start_time.elapsed().as_secs_f64()
    }
}
