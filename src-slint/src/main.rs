//! x-doc 核心链路压测 —— Slint GUI（纯 Rust，无 webkit，兼容 glibc 2.28）

slint::include_modules!();

use slint::Model;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use stress_core::engine;
use stress_core::types::{RunHandle, StressConfig};

const COPY_HEADER: &str = "#\t文件\t页数\t上传\t就绪\t下载\t转换\t首页\t全页\t端到端\t状态";

/// 数值格式化：空 → —，≥1000 → x.xs，否则 xms
fn vt(v: &serde_json::Value) -> String {
    match v.as_u64() {
        None => "—".into(),
        Some(ms) if ms >= 1000 => format!("{:.1}s", ms as f64 / 1000.0),
        Some(ms) => format!("{}ms", ms),
    }
}

/// 页数
fn vp(v: &serde_json::Value) -> String {
    v.as_u64().map(|n| n.to_string()).unwrap_or_else(|| "—".into())
}

/// 去掉 engine 侧加的 core-{run}-{seq}- 前缀
fn strip_prefix(name: &str) -> String {
    if let Some(rest) = name.strip_prefix("core-") {
        let mut parts = rest.splitn(3, '-');
        if let (Some(a), Some(b), Some(c)) = (parts.next(), parts.next(), parts.next()) {
            let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
            if digits(a) && digits(b) && !c.is_empty() {
                return c.to_string();
            }
        }
    }
    name.to_string()
}

/// 文件大小格式化
fn fmt_size(n: u64) -> String {
    if n >= 1048576 {
        format!("{:.1}MB", n as f64 / 1048576.0)
    } else if n >= 1024 {
        format!("{:.1}KB", n as f64 / 1024.0)
    } else {
        format!("{}B", n)
    }
}

fn main() {
    let app = App::new().unwrap();

    // 共享状态
    let running = Arc::new(AtomicBool::new(false));
    let handle = Arc::new(RunHandle::new());
    // 结果行镜像（轮询线程更新；复制/CSV 在 UI 线程读）
    let rows_mirror: Arc<Mutex<Vec<(ResultRowUi, String)>>> = Arc::new(Mutex::new(Vec::new()));

    // 文件模型（UI 线程）
    let files_model = Rc::new(slint::VecModel::<FileItem>::from(Vec::new()));
    app.set_files(files_model.clone().into());

    let sync_all_checked = {
        let app = app.as_weak();
        let files_model = files_model.clone();
        move || {
            let mut n = 0usize;
            let mut checked = 0usize;
            for i in 0..files_model.row_count() {
                n += 1;
                if files_model.row_data(i).map(|f| f.checked).unwrap_or(false) {
                    checked += 1;
                }
            }
            if let Some(app) = app.upgrade() {
                app.set_all_checked(n > 0 && checked == n);
            }
        }
    };

    // ── 添加文件（去重、默认勾选） ──
    let add_files = {
        let app = app.as_weak();
        let files_model = files_model.clone();
        let sync = sync_all_checked.clone();
        move |paths: Vec<std::path::PathBuf>| {
            let Some(app) = app.upgrade() else { return };
            let existing: std::collections::HashSet<String> = (0..files_model.row_count())
                .filter_map(|i| files_model.row_data(i))
                .map(|f| f.path.to_string())
                .collect();
            let mut added = 0usize;
            for p in paths {
                let path = p.to_string_lossy().to_string();
                if existing.contains(&path) {
                    continue;
                }
                let name =
                    p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| path.clone());
                let size = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
                files_model.push(FileItem {
                    name: name.into(),
                    path: path.into(),
                    size: fmt_size(size).into(),
                    checked: true,
                });
                added += 1;
            }
            if added > 0 {
                sync();
            } else {
                app.set_status_text("所选文件已全部添加".into());
            }
        }
    };

    // ── 文件夹 / 多选文件（rfd 原生对话框，UI 线程同步弹出） ──
    {
        let add = add_files.clone();
        let app = app.as_weak();
        app.unwrap().on_pick_folder(move || {
            if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                let mut paths: Vec<std::path::PathBuf> = std::fs::read_dir(&dir)
                    .map(|entries| {
                        entries
                            .flatten()
                            .map(|e| e.path())
                            .filter(|p| {
                                p.is_file()
                                    && !p.file_name().map(|n| n.to_string_lossy().starts_with('.')).unwrap_or(false)
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                paths.sort();
                if paths.is_empty() {
                    if let Some(app) = app.upgrade() {
                        app.set_status_text("该文件夹没有可用文件".into());
                    }
                } else {
                    add(paths);
                }
            }
        });
    }
    {
        let add = add_files.clone();
        app.on_pick_files(move || {
            if let Some(sel) = rfd::FileDialog::new().pick_files() {
                add(sel);
            }
        });
    }

    // ── 清空 / 全选 / 勾选同步 ──
    {
        let app = app.as_weak();
        let files_model = files_model.clone();
        app.unwrap().on_clear_files(move || {
            files_model.clear();
            if let Some(app) = app.upgrade() {
                app.set_all_checked(false);
                app.set_status_text("就绪".into());
            }
        });
    }
    {
        let app = app.as_weak();
        let files_model = files_model.clone();
        app.unwrap().on_toggle_all(move |checked| {
            for i in 0..files_model.row_count() {
                if let Some(mut f) = files_model.row_data(i) {
                    f.checked = checked;
                    files_model.set_row_data(i, f);
                }
            }
            if let Some(app) = app.upgrade() {
                app.set_all_checked(checked);
            }
        });
    }
    {
        let sync = sync_all_checked.clone();
        app.on_files_changed(move || {
            sync();
        });
    }

    // ── 复制结果（TSV → 剪贴板） ──
    {
        let rows_mirror = rows_mirror.clone();
        let app = app.as_weak();
        app.unwrap().on_copy_results(move || {
            let Some(app) = app.upgrade() else { return };
            let rows = rows_mirror.lock().unwrap();
            if rows.is_empty() {
                app.set_status_text("暂无结果可复制".into());
                return;
            }
            let mut lines = vec![COPY_HEADER.to_string()];
            for (i, (r, err)) in rows.iter().enumerate() {
                let status = if r.state == 2 { format!("✗ {err}") } else if r.state == 1 { "✓".into() } else { "●".into() };
                lines.push(format!(
                    "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                    r.no, r.file_name, r.page_count, r.upload, r.ready, r.download, r.convert, r.first, r.full, r.e2e, status
                ));
            }
            let text = lines.join("\n");
            match arboard::Clipboard::new().and_then(|mut cb| cb.set_text(text)) {
                Ok(_) => app.set_status_text("已复制到剪贴板".into()),
                Err(e) => app.set_status_text(format!("复制失败: {e}").into()),
            }
        });
    }

    // ── 导出 CSV ──
    {
        let rows_mirror = rows_mirror.clone();
        let app = app.as_weak();
        app.unwrap().on_export_csv(move || {
            let Some(app) = app.upgrade() else { return };
            let rows = rows_mirror.lock().unwrap();
            if rows.is_empty() {
                app.set_status_text("暂无结果可导出".into());
                return;
            }
            let mut csv = String::from("序号,文件,页数,上传,就绪,下载,转换,首页,全页,端到端,状态,错误\n");
            for (i, (r, err)) in rows.iter().enumerate() {
                let status = if r.state == 2 { "失败" } else if r.state == 1 { "完成" } else { "预热" };
                let err = err.replace(',', "，");
                csv.push_str(&format!(
                    "{},{},{},{},{},{},{},{},{},{},{},{}\n",
                    i + 1, r.file_name, r.page_count, r.upload, r.ready, r.download, r.convert, r.first, r.full, r.e2e, status, err
                ));
            }
            if let Some(path) =
                rfd::FileDialog::new().set_file_name("core-flow.csv").save_file()
            {
                match std::fs::write(&path, csv) {
                    Ok(_) => app.set_status_text(format!("已导出 {}", path.display()).into()),
                    Err(e) => app.set_status_text(format!("写入失败: {e}").into()),
                }
            }
        });
    }

    // ── 开始测试 ──
    {
        let app = app.as_weak();
        let running = running.clone();
        let handle = handle.clone();
        let files_model = files_model.clone();
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
            let files: Vec<String> = (0..files_model.row_count())
                .filter_map(|i| files_model.row_data(i))
                .filter(|f| f.checked)
                .map(|f| f.path.to_string())
                .collect();
            if files.is_empty() {
                app.set_status_text("请先添加并勾选文件".into());
                return;
            }
            let headers_raw = app.get_auth_headers().trim().to_string();
            let auth_headers = if headers_raw.is_empty() {
                serde_json::json!({})
            } else {
                match serde_json::from_str(&headers_raw) {
                    Ok(v) => v,
                    Err(_) => {
                        app.set_status_text("AUTH_HEADERS 不是合法 JSON".into());
                        return;
                    }
                }
            };

            running.store(true, Ordering::Relaxed);
            app.set_running(true);
            app.set_status_text(format!("运行中… 0/{}", files.len()).into());
            app.set_log_text("".into());

            let config = StressConfig {
                base_url,
                auth_token: app.get_auth_token().trim().to_string(),
                auth_headers,
                steps: vec!["upload".into(), "convert".into(), "range".into(), "excel".into()],
                concurrency: app.get_concurrency().trim().parse::<u32>().unwrap_or(1).max(1),
                repeats: app.get_repeats().trim().parse::<u32>().unwrap_or(2).max(1),
                range_chunk: app.get_range_chunk().trim().parse::<u64>().unwrap_or(1048576).max(1),
                tile_batch: app.get_tile_batch().trim().parse::<usize>().unwrap_or(10).max(1),
                poll_interval_ms: app.get_poll_interval().trim().parse::<u64>().unwrap_or(1000).max(100),
                poll_max_times: app.get_poll_max().trim().parse::<u32>().unwrap_or(600).max(1),
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
        let handle = handle.clone();
        app.on_stop_test(move || {
            handle.stop_flag.store(true, Ordering::Relaxed);
        });
    }

    // ── 轮询状态更新 UI ──
    {
        let app_weak = app.as_weak();
        let running = running.clone();
        let handle = handle.clone();
        let rows_mirror = rows_mirror.clone();
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

                // 结果行 → UI 行
                let ui_rows: Vec<(ResultRowUi, String)> = rows
                    .iter()
                    .map(|r| {
                        let state = if r.warmup {
                            0
                        } else if r.success {
                            1
                        } else {
                            2
                        };
                        (
                            ResultRowUi {
                                no: String::new().into(), // 序号在下面 enumerate 填
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
                    })
                    .enumerate()
                    .map(|(i, (mut ui, err))| {
                        ui.no = (i + 1).to_string().into();
                        (ui, err)
                    })
                    .collect();

                let success = rows.iter().filter(|r| r.success).count();
                let total = rows.len();
                let summary = summary.map(|s| (s.success, s.total, s.files_per_min));

                // 日志增量的判断在轮询线程完成，避免跨线程可变借用
                let (new_count, log_text) = if logs.len() != last_log_count {
                    let text = if logs.is_empty() { None } else { Some(logs.join("\n")) };
                    let new_count = logs.len();
                    last_log_count = logs.len();
                    (new_count, text)
                } else {
                    (logs.len(), None)
                };

                let mirror = ui_rows.clone();
                let _ = app_weak.upgrade_in_event_loop(move |app| {
                    app.set_results(
                        std::rc::Rc::new(slint::VecModel::from(mirror.into_iter().map(|(r, _)| r).collect::<Vec<_>>()))
                            .into(),
                    );
                    if let Some(text) = log_text {
                        app.set_log_text(text.into());
                    }
                    let _ = new_count;
                    if is_running {
                        app.set_status_text(format!("运行中… {}/{}", success, total).into());
                        app.set_running(true);
                    } else if let Some((s_ok, s_total, fpm)) = summary {
                        if s_total > 0 {
                            app.set_status_text(
                                format!("{}/{} 成功 · 吞吐 {:.1} 件/min", s_ok, s_total, fpm).into(),
                            );
                        }
                        app.set_running(false);
                    } else {
                        app.set_running(false);
                    }
                });

                // 更新镜像
                {
                    let mut m = rows_mirror.lock().unwrap();
                    *m = ui_rows;
                }
            }
        });
    }

    app.run().unwrap();
}
