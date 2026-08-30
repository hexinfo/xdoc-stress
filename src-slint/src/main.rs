//! x-doc 核心链路压测 —— Slint GUI（纯 Rust，无 webkit，兼容 glibc 2.28）

#![cfg_attr(all(not(debug_assertions), target_os = "windows"), windows_subsystem = "windows")]

slint::include_modules!();

use slint::Model;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use stress_core::engine;
use stress_core::types::{ResultRow, RunHandle, StressConfig};

const CSV_HEADER: &str = "序号,文件,页数,上传,就绪,下载,转换,首页,全页,端到端,状态,错误";
const COPY_HEADER: &str = "#\t文件\t页数\t上传\t就绪\t下载\t转换\t首页\t全页\t端到端\t状态";

// ── 格式化工具 ──

fn vt(v: &serde_json::Value) -> String {
    match v.as_u64() {
        Some(ms) if ms >= 1000 => format!("{:.1}s", ms as f64 / 1000.0),
        Some(ms) => format!("{ms}ms"),
        None => "—".into(),
    }
}

fn vp(v: &serde_json::Value) -> String {
    v.as_u64().map(|n| n.to_string()).unwrap_or_else(|| "—".into())
}

fn fmt_size(n: u64) -> String {
    if n >= 1048576 { format!("{:.1}MB", n as f64 / 1048576.0) }
    else if n >= 1024 { format!("{:.1}KB", n as f64 / 1024.0) }
    else { format!("{n}B") }
}

/// 去掉 engine 侧的 core-{runId}-{seq}- 前缀，还原源文件名
fn strip_prefix(name: &str) -> String {
    name.strip_prefix("core-")
        .and_then(|rest| {
            let mut parts = rest.splitn(3, '-');
            match (parts.next(), parts.next(), parts.next()) {
                (Some(a), Some(b), Some(c)) if !a.is_empty() && !b.is_empty() && !c.is_empty()
                    && a.bytes().all(|b| b.is_ascii_digit())
                    && b.bytes().all(|b| b.is_ascii_digit()) => Some(c.to_string()),
                _ => None,
            }
        })
        .unwrap_or_else(|| name.to_string())
}

/// ResultRow → UI 行
fn to_ui_row(r: &ResultRow, seq: usize) -> (ResultRowUi, String) {
    let state = if r.warmup { 0 } else if r.success { 1 } else { 2 };
    (
        ResultRowUi {
            no: seq.to_string().into(),
            file_name: strip_prefix(&r.file_name).into(),
            page_count: vp(&r.page_count).into(),
            upload: vt(&r.upload_ms).into(),
            ready: vt(&r.state_ready_ms).into(),
            download: vt(&r.metric_download_ms).into(),
            convert: vt(&r.metric_convert_ms).into(),
            first: vt(&r.first_range_ms).into(),
            full: vt(&r.full_ms).into(),
            e2e: vt(&r.e2e_ms).into(),
            state,
            failed: !r.warmup && !r.success,
        },
        r.error.clone(),
    )
}

fn main() {
    let app = App::new().unwrap();
    let running = Arc::new(AtomicBool::new(false));
    let handle = Arc::new(RunHandle::new());
    let rows_mirror: Arc<Mutex<Vec<(ResultRowUi, String)>>> = Arc::new(Mutex::new(Vec::new()));
    let files_model = Rc::new(slint::VecModel::<FileItem>::from(Vec::new()));
    app.set_files(files_model.clone().into());

    // ── 文件列表操作 ──

    let sync_all_checked = {
        let app = app.as_weak();
        let model = files_model.clone();
        move || {
            let n = model.row_count();
            let checked = (0..n).filter(|&i| model.row_data(i).map(|f| f.checked).unwrap_or(false)).count();
            if let Some(app) = app.upgrade() {
                app.set_all_checked(n > 0 && checked == n);
            }
        }
    };

    let add_files = {
        let app = app.as_weak();
        let model = files_model.clone();
        let sync = sync_all_checked.clone();
        move |paths: Vec<std::path::PathBuf>| {
            let existing: std::collections::HashSet<String> = (0..model.row_count())
                .filter_map(|i| model.row_data(i))
                .map(|f| f.path.to_string())
                .collect();
            let added = paths.into_iter()
                .filter(|p| {
                    let path = p.to_string_lossy().to_string();
                    !existing.contains(&path)
                })
                .map(|p| {
                    let path = p.to_string_lossy().to_string();
                    let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| path.clone());
                    let size = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
                    FileItem { name: name.into(), path: path.into(), size: fmt_size(size).into(), checked: true }
                })
                .count_of_push(&model);
            if added > 0 { sync(); }
            else if let Some(app) = app.upgrade() { app.set_status_text("所选文件已全部添加".into()); }
        }
    };

    // trait 帮 add_files 里的 push 循环计数
    trait PushCount {
        fn count_of_push(self, model: &slint::VecModel<FileItem>) -> usize;
    }
    impl<I: Iterator<Item = FileItem>> PushCount for I {
        fn count_of_push(self, model: &slint::VecModel<FileItem>) -> usize {
            self.enumerate().map(|(i, item)| { model.push(item); i + 1 }).last().unwrap_or(0)
        }
    }

    // ── 文件选择器回调 ──

    {
        let add = add_files.clone();
        let weak = app.as_weak();
        app.on_pick_folder(move || {
            if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                let mut paths: Vec<_> = std::fs::read_dir(&dir)
                    .map(|entries| entries.flatten().map(|e| e.path())
                        .filter(|p| p.is_file() && !p.file_name().map(|n| n.to_string_lossy().starts_with('.')).unwrap_or(false))
                        .collect())
                    .unwrap_or_default();
                paths.sort();
                if paths.is_empty() {
                    if let Some(app) = weak.upgrade() { app.set_status_text("该文件夹没有可用文件".into()); }
                } else {
                    add(paths);
                }
            }
        });
    }
    {
        let add = add_files.clone();
        app.on_pick_files(move || {
            if let Some(sel) = rfd::FileDialog::new().pick_files() { add(sel); }
        });
    }
    {
        let weak = app.as_weak();
        let model = files_model.clone();
        app.on_clear_files(move || {
            model.clear();
            if let Some(app) = weak.upgrade() {
                app.set_all_checked(false);
                app.set_status_text("就绪".into());
            }
        });
    }
    {
        let weak = app.as_weak();
        let model = files_model.clone();
        app.on_toggle_all(move |checked| {
            for i in 0..model.row_count() {
                if let Some(mut f) = model.row_data(i) {
                    f.checked = checked;
                    model.set_row_data(i, f);
                }
            }
            if let Some(app) = weak.upgrade() { app.set_all_checked(checked); }
        });
    }
    {
        let sync = sync_all_checked.clone();
        app.on_files_changed(move || sync());
    }

    // ── 复制结果 ──
    {
        let mirror = rows_mirror.clone();
        let weak = app.as_weak();
        app.on_copy_results(move || {
            let Some(app) = weak.upgrade() else { return };
            let rows = mirror.lock().unwrap();
            if rows.is_empty() { app.set_status_text("暂无结果可复制".into()); return; }
            let mut lines = vec![COPY_HEADER.to_string()];
            for (r, err) in rows.iter() {
                let status = match r.state { 2 => format!("✗ {err}"), 1 => "✓".into(), _ => "●".into() };
                lines.push(format!("{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                    r.no, r.file_name, r.page_count, r.upload, r.ready, r.download, r.convert, r.first, r.full, r.e2e, status));
            }
            match arboard::Clipboard::new().and_then(|mut cb| cb.set_text(lines.join("\n"))) {
                Ok(_) => app.set_status_text("已复制到剪贴板".into()),
                Err(e) => app.set_status_text(format!("复制失败: {e}").into()),
            }
        });
    }

    // ── 导出 CSV ──
    {
        let mirror = rows_mirror.clone();
        let weak = app.as_weak();
        app.on_export_csv(move || {
            let Some(app) = weak.upgrade() else { return };
            let rows = mirror.lock().unwrap();
            if rows.is_empty() { app.set_status_text("暂无结果可导出".into()); return; }
            let mut csv = format!("{CSV_HEADER}\n");
            for (i, (r, err)) in rows.iter().enumerate() {
                let status = match r.state { 2 => "失败", 1 => "完成", _ => "预热" };
                csv.push_str(&format!("{},{},{},{},{},{},{},{},{},{},{},{}\n",
                    i + 1, r.file_name, r.page_count, r.upload, r.ready, r.download, r.convert, r.first, r.full, r.e2e, status, err.replace(',', "，")));
            }
            drop(rows);
            if let Some(path) = rfd::FileDialog::new().set_file_name("core-flow.csv").save_file() {
                match std::fs::write(&path, csv) {
                    Ok(_) => app.set_status_text(format!("已导出 {}", path.display()).into()),
                    Err(e) => app.set_status_text(format!("写入失败: {e}").into()),
                }
            }
        });
    }

    // ── 开始测试 ──
    {
        let weak = app.as_weak();
        let running = running.clone();
        let handle = handle.clone();
        let model = files_model.clone();
        app.on_start_test(move || {
            if running.load(Ordering::Relaxed) { return; }
            let Some(app) = weak.upgrade() else { return };

            let files: Vec<String> = (0..model.row_count())
                .filter_map(|i| model.row_data(i))
                .filter(|f| f.checked)
                .map(|f| f.path.to_string())
                .collect();
            if files.is_empty() { app.set_status_text("请先添加并勾选文件".into()); return; }

            let base_url = app.get_base_url().trim().to_string();
            if base_url.is_empty() { app.set_status_text("请填 BASE_URL".into()); return; }

            let headers_raw = app.get_auth_headers().trim().to_string();
            let auth_headers = if headers_raw.is_empty() {
                serde_json::json!({})
            } else {
                match serde_json::from_str(&headers_raw) {
                    Ok(v) => v,
                    Err(_) => { app.set_status_text("AUTH_HEADERS 不是合法 JSON".into()); return; }
                }
            };

            let parse = |s: slint::SharedString, d: &str| s.trim().parse::<f64>().map(|v| v.max(1.0)).unwrap_or_else(|_| d.parse().unwrap_or(1.0));
            let cfg = StressConfig {
                base_url,
                auth_token: app.get_auth_token().trim().to_string(),
                auth_headers,
                steps: vec!["upload".into(), "convert".into(), "range".into(), "excel".into()],
                concurrency: parse(app.get_concurrency(), "1") as u32,
                repeats: parse(app.get_repeats(), "2") as u32,
                range_chunk: parse(app.get_range_chunk(), "1048576") as u64,
                tile_batch: parse(app.get_tile_batch(), "10") as usize,
                poll_interval_ms: parse(app.get_poll_interval(), "1000").max(100.0) as u64,
                poll_max_times: parse(app.get_poll_max(), "600") as u32,
                files,
            };

            running.store(true, Ordering::Relaxed);
            app.set_running(true);
            app.set_status_text(format!("运行中… 0/{}", cfg.files.len()).into());
            app.set_log_text("".into());

            let running2 = running.clone();
            let handle2 = handle.clone();
            std::thread::spawn(move || {
                let rt = tokio::runtime::Runtime::new().unwrap();
                let stop = Arc::new(AtomicBool::new(false));
                let status = handle2.status.clone();
                let file_list: Vec<(String, String)> = cfg.files.iter()
                    .map(|p| (p.rsplit('/').next().unwrap_or(p).to_string(), p.clone()))
                    .collect();
                rt.block_on(async { engine::start_stress(cfg, stop, status, file_list).await; });
                running2.store(false, Ordering::Relaxed);
            });
        });
    }

    // ── 停止 ──
    {
        let handle = handle.clone();
        app.on_stop_test(move || handle.stop_flag.store(true, Ordering::Relaxed));
    }

    // ── 轮询更新 UI ──
    {
        let weak = app.as_weak();
        let running = running.clone();
        let handle = handle.clone();
        let mirror = rows_mirror.clone();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            let mut last_log = 0usize;
            loop {
                std::thread::sleep(Duration::from_millis(500));
                let is_running = running.load(Ordering::Relaxed);

                let (logs, rows, summary) = rt.block_on(async {
                    let st = handle.status.lock().await;
                    (st.logs.clone(), st.rows.clone(), st.summary.clone())
                });

                let ui_rows: Vec<_> = rows.iter().enumerate().map(|(i, r)| to_ui_row(r, i + 1)).collect();
                let success = rows.iter().filter(|r| r.success).count();
                let total = rows.len();
                let log_text = if logs.len() != last_log { last_log = logs.len(); Some(logs.join("\n")) } else { None };

                {
                    let mut m = mirror.lock().unwrap();
                    *m = ui_rows.clone();
                }

                let _ = weak.upgrade_in_event_loop(move |app| {
                    let model: slint::VecModel<ResultRowUi> = ui_rows.into_iter().map(|(r, _)| r).collect();
                    app.set_results(Rc::new(model).into());
                    if let Some(text) = log_text { app.set_log_text(text.into()); }
                    if is_running {
                        app.set_status_text(format!("运行中… {success}/{total}").into());
                        app.set_running(true);
                    } else {
                        if let Some(s) = &summary {
                            if s.total > 0 {
                                app.set_status_text(format!("{}/{} 成功 · 吞吐 {:.1} 件/min", s.success, s.total, s.files_per_min).into());
                            }
                        }
                        app.set_running(false);
                    }
                });
            }
        });
    }

    app.run().unwrap();
}
