use crate::client::{measure, StressClient};
use crate::types::*;
use serde_json::Value;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

/// 判断是否 Excel 族
fn is_workbook(name: &str) -> bool {
    name.to_lowercase().ends_with(".xls") || name.to_lowercase().ends_with(".xlsx") || name.to_lowercase().ends_with(".csv")
}

/// 单任务执行
async fn run_task(
    client: &StressClient,
    cfg: &StressConfig,
    file_path: &str,
    seq: usize,
    warmup: bool,
    run_id: i64,
    stop: &AtomicBool,
) -> ResultRow {
    let source_name = file_path.rsplit('/').next().unwrap_or(file_path);
    let chain = if is_workbook(source_name) { "excel" } else { "range" };
    let preview_type = if chain == "excel" { "ugz" } else { "pdf" };
    let file_name = format!("core-{}-{}-{}", run_id, seq, source_name);
    let steps: Vec<&str> = cfg.steps.iter().map(|s| s.as_str()).collect();
    let mut executed: Vec<String> = Vec::new();
    let t_start = std::time::Instant::now();

    let mut row = ResultRow {
        ts: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as i64,
        file_name: file_name.clone(),
        steps: String::new(),
        chain: chain.to_string(),
        page_count: Value::Null,
        upload_ms: Value::Null,
        prepare_ms: Value::Null,
        state_polls: Value::Null,
        state_ready_ms: Value::Null,
        metric_download_ms: Value::Null,
        metric_convert_ms: Value::Null,
        first_range_ms: Value::Null,
        full_ms: Value::Null,
        e2e_ms: Value::Null,
        detail: String::new(),
        success: false,
        error: String::new(),
        warmup,
    };

    let result = async {
        // ── 1. upload ──
        let (file_id, upload_ms) = measure(client.upload(file_path)).await;
        let file_id = file_id?;
        row.upload_ms = serde_json::json!(upload_ms);
        executed.push("upload".to_string());

        // ── 2. convert（prepare + state 轮询） ──
        let mut file_size: u64 = 0;
        if steps.contains(&"convert") {
            let (_, prepare_ms) = measure(client.prepare(&file_id, &file_name)).await;
            row.prepare_ms = serde_json::json!(prepare_ms);
            let t_ready = std::time::Instant::now();

            let mut polls: u32 = 0;
            loop {
                if stop.load(Ordering::Relaxed) {
                    return Err("已停止（用户请求）".to_string());
                }
                if polls >= cfg.poll_max_times {
                    return Err(format!("state 未就绪：polls={}（超过上限 {}）", polls, cfg.poll_max_times));
                }
                polls += 1;
                let data = client.state(&file_id, &file_name, preview_type).await?;
                let size = data["size"].as_u64().unwrap_or(0);
                let count = data["pageCount"].as_u64().unwrap_or(0);
                if size > 0 || count > 0 {
                    file_size = size;
                    row.page_count = serde_json::json!(count);
                    if let Some(m) = data["metric"].as_object() {
                        row.metric_download_ms = m.get("download").and_then(|v| v.as_i64()).map(|v| serde_json::json!(v)).unwrap_or(Value::Null);
                        row.metric_convert_ms = m.get("convert").and_then(|v| v.as_i64()).map(|v| serde_json::json!(v)).unwrap_or(Value::Null);
                    }
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(cfg.poll_interval_ms)).await;
            }
            row.state_polls = serde_json::json!(polls);
            row.state_ready_ms = serde_json::json!(t_ready.elapsed().as_millis());
            executed.push("convert".to_string());
        } else {
            return Err("未选 convert 暂不支持（首版简化）".to_string());
        }

        // ── 3/4. 拉取 ──
        let want_pull = if chain == "excel" { steps.contains(&"excel") } else { steps.contains(&"range") };
        if want_pull {
            executed.push(chain.to_string());
            let t_pull = std::time::Instant::now();
            if chain == "excel" {
                let structure = client.workbook_structure(&file_id, &file_name).await?;
                let order: Vec<String> = structure["sheetOrder"]
                    .as_array()
                    .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                    .unwrap_or_default();
                let mut tiles_want: usize = 0;
                let mut tiles_got: usize = 0;
                let mut tiles_ms: u64 = 0;
                for sheet_id in &order {
                    if stop.load(Ordering::Relaxed) {
                        return Err("已停止（用户请求）".to_string());
                    }
                    let manifest = client.tile_manifest(&file_id, &file_name, sheet_id).await?;
                    let ids: Vec<String> = manifest["tiles"]
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .filter_map(|t| t["tileId"].as_str().map(String::from))
                                .collect()
                        })
                        .unwrap_or_default();
                    tiles_want += ids.len();
                    for chunk in ids.chunks(cfg.tile_batch) {
                        if stop.load(Ordering::Relaxed) {
                            return Err("已停止（用户请求）".to_string());
                        }
                        let (got, ms) = measure(client.tile_data(&file_id, &file_name, sheet_id, chunk)).await;
                        let got = got?;
                        tiles_ms += ms;
                        tiles_got += got;
                    }
                }
                if tiles_got < tiles_want {
                    return Err(format!("tiles 不完整: 实收 {} < 清单 {}", tiles_got, tiles_want));
                }
                let tag = if warmup { "冷启动;" } else { "" };
                row.detail = format!("{}sheets={},tiles={}/{},tilesMs={}", tag, order.len(), tiles_got, tiles_want, tiles_ms);
                row.full_ms = serde_json::json!(t_pull.elapsed().as_millis());
            } else {
                let mut begin: u64 = 0;
                let mut chunks: u64 = 0;
                let mut bytes: u64 = 0;
                let mut first_ms: u64 = 0;
                while begin < file_size {
                    if stop.load(Ordering::Relaxed) {
                        return Err("已停止（用户请求）".to_string());
                    }
                    let end = std::cmp::min(begin + cfg.range_chunk, file_size);
                    let (n, ms) = measure(client.range(&file_id, &file_name, begin, end)).await;
                    let n = n?;
                    if chunks == 0 {
                        first_ms = ms;  // 首页指标 = state 就绪后第一个 range 请求的耗时
                    }
                    bytes += n;
                    chunks += 1;
                    begin = end;
                }
                let _ = first_ms;
                let tag = if warmup { "冷启动;" } else { "" };
                row.first_range_ms = serde_json::json!(first_ms);
                row.detail = format!("{}chunks={},bytes={},chunkSize={}", tag, chunks, bytes, cfg.range_chunk);
                row.full_ms = serde_json::json!(t_pull.elapsed().as_millis());
            }
        } else {
            row.detail = format!("未选拉取步骤（{} 族需要 {}）", chain, chain);
        }

        row.e2e_ms = serde_json::json!(t_start.elapsed().as_millis());
        Ok(())
    };

    match result.await {
        Ok(()) => {
            row.steps = executed.join(",");
            row.success = true;
        }
        Err(e) => {
            row.steps = executed.join(",");
            row.error = e;
        }
    }
    row
}

/// 启动压测（tokio 任务池 + 抢活派发）
pub async fn start_stress(
    cfg: StressConfig,
    stop_flag: Arc<AtomicBool>,
    status: Arc<tokio::sync::Mutex<RunStatus>>,
    file_contents: Vec<(String, String)>, // (name, abs_path)
) {
    let threads = std::cmp::min(cfg.concurrency as usize, file_contents.len() * cfg.repeats as usize).max(1);
    let run_id = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as i64;

    {
        let mut st = status.lock().await;
        st.running = true;
        st.stop_requested = false;
        st.logs.clear();
        st.rows.clear();
        st.summary = None;
        st.logs.push(format!(
            "文件 {} 个 × repeats={} ，并发 {}，步骤 {}",
            file_contents.len(),
            cfg.repeats,
            threads,
            cfg.steps.join(",")
        ));
    }

    let client = match StressClient::new(&cfg.base_url, &cfg.auth_token, &cfg.auth_headers.to_string()) {
        Ok(c) => c,
        Err(e) => {
            let mut st = status.lock().await;
            st.logs.push(format!("❌ {}", e));
            st.running = false;
            return;
        }
    };

    // 展开任务清单
    let mut tasks: Vec<(String, bool)> = Vec::new(); // (abs_path, warmup)
    for (name, path) in &file_contents {
        for r in 0..cfg.repeats {
            tasks.push((path.clone(), r == 0));
        }
    }
    let total = tasks.len();
    let next = Arc::new(AtomicUsize::new(0));
    let cfg = Arc::new(cfg);
    let client = Arc::new(client);

    let start_ms = std::time::Instant::now();
    let mut handles = Vec::new();

    for _thread_id in 0..threads {
        let next = next.clone();
        let tasks = tasks.clone();
        let cfg = cfg.clone();
        let client = client.clone();
        let stop = stop_flag.clone();
        let status = status.clone();
        let run_id = run_id;

        handles.push(tokio::spawn(async move {
            loop {
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                let idx = next.fetch_add(1, Ordering::SeqCst);
                if idx >= tasks.len() {
                    break;
                }
                let (path, warmup) = &tasks[idx];
                let row = run_task(&client, &cfg, path, idx + 1, *warmup, run_id, &stop).await;

                let name = path.rsplit('/').next().unwrap_or(path);
                let icon = if row.warmup { "🔵" } else if row.success { "✅" } else { "❌" };

                let mut st = status.lock().await;
                st.logs.push(format!(
                    "{} {}#{}: [{}] upload={} ready={} 全页={} {}",
                    icon,
                    name,
                    idx + 1,
                    row.steps,
                    row.upload_ms.as_i64().map(|v| format!("{}ms", v)).unwrap_or("-".to_string()),
                    row.state_ready_ms.as_i64().map(|v| format!("{}ms", v)).unwrap_or("-".to_string()),
                    row.full_ms.as_i64().map(|v| format!("{}ms", v)).unwrap_or("-".to_string()),
                    row.detail
                ));
                st.rows.push(row);
                drop(st);
            }
        }));
    }

    for h in handles {
        let _ = h.await;
    }

    let wall_ms = start_ms.elapsed().as_millis() as u64;
    let st0 = status.lock().await;
    let success = {
      let ok_all: Vec<_> = st0.rows.iter().filter(|r| r.success).collect();
      let ok_stable: Vec<_> = ok_all.iter().filter(|r| !r.warmup).collect();
      if ok_stable.is_empty() { ok_all.len() } else { ok_stable.len() }
    };
    drop(st0);
    let files_per_min = if wall_ms > 0 { (success as f64) * 60000.0 / wall_ms as f64 } else { 0.0 };

    let mut st = status.lock().await;
    st.running = false;
    st.summary = Some(RunSummary {
        total,
        success,
        concurrency: threads as u32,
        wall_ms,
        files_per_min: (files_per_min * 100.0).round() / 100.0,
    });
    st.logs.push(format!("━━━ 完成：{}/{} 成功，吞吐 {:.2} 件/分钟 ━━━", success, total, files_per_min));
}
