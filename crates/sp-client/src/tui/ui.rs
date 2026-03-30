use ratatui::Frame;

use super::app::{App, View};
use super::views::{detail, package_list};

pub fn draw(f: &mut Frame, app: &App) {
    let area = f.area();

    match &app.view {
        View::PackageList => package_list::draw(f, app, area),
        View::Detail { index } => {
            let key = app
                .packages
                .get(*index)
                .map(|p| (p.name.clone(), p.version.clone()));
            let details = key.as_ref().and_then(|k| app.detail_cache.get(k));
            detail::draw(f, app, details, area);
        }
    }
}
