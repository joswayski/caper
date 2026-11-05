//! The daily mascot icon (`caper_client::daily_icon`), saved in eframe storage.

pub use caper_client::daily_icon::DailyIcon;

pub fn load(storage: Option<&dyn eframe::Storage>) -> DailyIcon {
    let saved_day = storage.and_then(|value| value.get_string("daily-icon-day-v1"));
    let saved_index = storage
        .and_then(|value| value.get_string("daily-icon-index-v1"))
        .and_then(|value| value.parse().ok());
    DailyIcon::load(saved_day.as_deref(), saved_index)
}
