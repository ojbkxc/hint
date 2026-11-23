// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // 语音 worker 由本进程自唤（`WorkerClient::spawn(current_exe, ...)`）。
    // 必须在任何 Tauri 初始化之前处理：worker 不创建窗口、不启动事件循环。
    if std::env::args().any(|argument| argument == hint_stt::WORKER_FLAG) {
        std::process::exit(match hint_stt::run_worker_from_env() {
            Ok(()) => 0,
            Err(error) => {
                eprintln!("voice worker: {error}");
                1
            }
        });
    }

    // `--doctor`：不启动界面，只收集诊断信息。同样必须在 Tauri 初始化前，
    // 因为它要能在「界面打不开」时还能跑通。
    if std::env::args().any(|argument| argument == "--doctor") {
        std::process::exit(hint_lib::doctor_entry());
    }

    hint_lib::run()
}
