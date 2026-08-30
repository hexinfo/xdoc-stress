//! x-doc 核心链路压测 —— Slint GUI（纯 Rust，无 webkit，兼容 glibc 2.28）

slint::include_modules!();

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use stress_core::engine;
use stress_core::types::{RunHandle, StressConfig};

fn main() {
    let app = App::new().unwrap();

    // 共享状态
    let running = Arc::new(AtomicBool::new(false));
    let handle = Arc::new(RunHandle::new());

    // ── 开始测试 ──
    {
        let app = app.as_weak();
        let running = running.clone();
        let handle = handle.clone();
        app.unwrap().on_start_test(move || {
            if running.load(Ordering::Relaxed) {
                return;
            }
            let app = app.unwrap();
            let base_url = app.get_base_url().trim().to_string();
            if base_url.is_empty() {
                app.set_status_text("请填 BASE_URL".into());
                return;
            }
            let auth_token = app.get_auth_token().trim().to_string();
            let concurrency = app.get_concurrency().trim().parse::<u32>().unwrap_or(1).max(1);
            let repeats = app.get_repeats().trim().parse::<u32>().unwrap_or(2).max(1);

            // 扫描当前目录的 fixtures/（简化版；后续可加文件选择器）
            let mut files = Vec::new();
            if let Ok(entries) = std::fs::read_dir("fixtures") {
                for e in entries.flatten() {
                    let name = e.file_name().to_string_lossy().to_string();
                    if !name.starts_with('.') && e.metadata().map(|m| m.is_file()).unwrap_or(false) {
                        files.push(e.path().to_string_lossy().to_string());
                    }
                }
            }
            if files.is_empty() {
                app.set_status_text("fixtures/ 目录无文件".into());
                return;
            }

            running.store(true, Ordering::Relaxed);
            app.set_running(true);
            app.set_status_text(format!("运行中… {} 个文件", files.len()).into());
            app.set_log_text("".into());

            let config = StressConfig {
                base_url,
                auth_token,
                auth_headers: serde_json::json!({}),
                steps: vec!["upload".into(), "convert".into(), "range".into(), "excel".into()],
                concurrency,
                repeats,
                range_chunk: 1048576,
                tile_batch: 10,
                poll_interval_ms: 1000,
                poll_max_times: 600,
                files,
            };

            let running2 = running.clone();
            let handle2 = handle.clone();
            std::thread::spawn(move || {
                let rt = tokio::runtime::Runtime::new().unwrap();
                let stop = Arc::new(AtomicBool::new(false));
                let status = handle2.status.clone();
                let file_list: Vec<(String, String)> = config
                    .files
                    .iter()
                    .map(|p| (p.rsplit('/').next().unwrap_or(p).to_string(), p.clone()))
                    .collect();

                rt.block_on(async {
                    engine::start_stress(config, stop, status, file_list).await;
                });

                running2.store(false, Ordering::Relaxed);
            });
        });
    }

    // ── 停止 ──
    {
        let app = app.as_weak();
        let handle = handle.clone();
        app.unwrap().on_stop_test(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                handle.stop_flag.store(true, Ordering::Relaxed);
            });
        });
    }

    // ── 轮询状态更新 UI ──
    {
        let app_weak = app.as_weak();
        let running = running.clone();
        let handle = handle.clone();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            let mut last_log_count = 0usize;
            loop {
                std::thread::sleep(Duration::from_millis(500));
                let is_running = running.load(Ordering::Relaxed);

                let (logs, rows, summary) = rt.block_on(async {
                    let st = handle.status.lock().await;
                    (st.logs.clone(), st.rows.clone(), st.summary.clone())
                });

                let app = if let Some(a) = app_weak.upgrade() { a } else { break };

                if logs.len() != last_log_count {
                    last_log_count = logs.len();
                    app.set_log_text(logs.join("\n").into());
                }
                app.set_row_count(rows.len() as i32);
                app.set_success_count(rows.iter().filter(|r| r.success).count() as i32);
                    app.set_result_text(format!("{}/{}", rows.iter().filter(|r| r.success).count(), rows.len()).into());

                if !is_running && rows.len() > 0 {
                    if let Some(s) = summary {
                        app.set_status_text(
                            format!("{}/{} 成功 · 吞吐 {:.1} 件/min", s.success, s.total, s.files_per_min).into(),
                        );
                    }
                    app.set_running(false);
                } else if is_running {
                    app.set_running(true);
                    app.set_status_text(format!("运行中… {}/{}", rows.iter().filter(|r| r.success).count(), rows.len()).into());
                }
            }
        });
    }

    app.run().unwrap();
}
