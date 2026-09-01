mod temps;
#[cfg(windows)]
mod elevate;
#[cfg(windows)]
mod lhm;

use temps::ThermalReading;

#[tauri::command]
async fn read_temperatures() -> ThermalReading {
    tauri::async_runtime::spawn_blocking(temps::read_temperatures)
        .await
        .unwrap_or_default()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg(all(windows, not(debug_assertions)))]
    elevate::ensure_admin();

    #[cfg(windows)]
    std::thread::spawn(|| lhm::warmup());

    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![read_temperatures])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
