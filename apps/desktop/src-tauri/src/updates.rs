use std::time::Duration;

use tauri::AppHandle;
use tauri_plugin_updater::UpdaterExt;

const INITIAL_CHECK_DELAY: Duration = Duration::from_secs(15);
const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

pub fn initialize(app: &AppHandle) {
    if cfg!(debug_assertions) || option_env!("CAPER_RELEASE_CHANNEL") != Some("preview") {
        return;
    }

    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(INITIAL_CHECK_DELAY).await;

        loop {
            if let Err(error) = install_available_update(&app).await {
                eprintln!("automatic update failed: {error}");
            }
            tokio::time::sleep(CHECK_INTERVAL).await;
        }
    });
}

async fn install_available_update(app: &AppHandle) -> tauri_plugin_updater::Result<()> {
    let Some(update) = app.updater()?.check().await? else {
        return Ok(());
    };

    update.download_and_install(|_, _| {}, || {}).await?;
    app.restart();
}
