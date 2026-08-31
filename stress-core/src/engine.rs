use crate::client::{measure, StressClient};
use crate::types::*;
use serde_json::Value;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

/// 判断是否 Excel 族（走瓦片链路）
fn is_workbook(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.ends_with(".xls") || lower.ends_with(".xlsx") || lower.ends_with(".csv")
}

/// 格式化毫秒为人类可读（≥1s 显示 x.xs）
pub fn fmt_ms(ms: u64) -> String {
    if ms >= 1000 { format!("{:.1}s", ms as f64 / 1000.0) } else { format!("{}ms", ms) }
}

/// JSON Value → Option<i64>（从 serde_json::Value 提取数值）
fn vi64(v: &Value) -> Option<i64> { v.as_i64() }

/// 单任务：range 全页拉取（分段到尾部），返回 (detail, full_ms, first_range_ms)
async fn pull_range(
    client: &StressClient,
    file_id: &str,
    file_name: &str,
    file_size: u64,
    chunk_size: u64,
    stop: &AtomicBool,
    warmup: bool,
) -> Result<(String, u64, u64), String> {
    let t = std::time::Instant::now();
    let mut begin = 0u64;
    let mut chunks = 0u64;
    let mut bytes = 0u64;
    let mut first_ms = 0u64;
    while begin < file_size {
        if stop.load(Ordering::Relaxed) { return Err(STOPPED.into()); }
        let end = begin.saturating_add(chunk_size).min(file_size);
        let (n, ms) = measure(client.range(file_id, file_name, begin, end)).await;
        let n = n?;
        if chunks == 0 { first_ms = ms; }
        bytes += n;
        chunks += 1;
        begin = end;
    }
    let tag = if warmup { "冷启动;" } else { "" };
    Ok((format!("{}chunks={},bytes={}", tag, chunks, bytes), t.elapsed().as_millis() as u64, first_ms))
}

/// 单任务：Excel 瓦片全量拉取
async fn pull_excel(
    client: &StressClient,
    file_id: &str,
    file_name: &str,
    tile_batch: usize,
    stop: &AtomicBool,
    warmup: bool,
) -> Result<(String, u64), String> {
    let t = std::time::Instant::now();
    let structure = client.workbook_structure(file_id, file_name).await?;
    let order: Vec<String> = structure["sheetOrder"].as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();
    let mut want = 0usize;
    let mut got = 0usize;
    for sheet_id in &order {
        if stop.load(Ordering::Relaxed) { return Err(STOPPED.into()); }
        let manifest = client.tile_manifest(file_id, file_name, sheet_id).await?;
        let ids: Vec<String> = manifest["tiles"].as_array()
            .map(|a| a.iter().filter_map(|x| x["tileId"].as_str().map(String::from)).collect())
            .unwrap_or_default();
        want += ids.len();
        for chunk in ids.chunks(tile_batch) {
            if stop.load(Ordering::Relaxed) { return Err(STOPPED.into()); }
            let (n, _) = measure(client.tile_data(file_id, file_name, sheet_id, chunk)).await;
            got += n?;
        }
    }
    if got < want { return Err(format!("tiles 不完整: {}/{}", got, want)); }
    let tag = if warmup { "冷启动;" } else { "" };
    Ok((format!("{}sheets={},tiles={}/{}", tag, order.len(), got, want), t.elapsed().as_millis() as u64))
}

const STOPPED: &str = "已停止";

/// 单任务执行：上传 → prepare → state 轮询 → range/excel 拉取
async fn run_task(
    client: &StressClient,
    cfg: &StressConfig,
    file_path: &str,
    seq: usize,
    warmup: bool,
    run_id: i64,
    stop: &AtomicBool,
) -> ResultRow {
    let name = file_path.rsplit('/').next().unwrap_or(file_path);
    let workbook = is_workbook(name);
    let chain = if workbook { "excel" } else { "range" };
    let preview = if workbook { "ugz" } else { "pdf" };
    let biz_name = format!("core-{}-{}-{}", run_id, seq, name);
    let t_start = std::time::Instant::now();
    let now_ms = || std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as i64;

    let mut row = ResultRow {
        ts: now_ms(), file_name: biz_name.clone(), steps: String::new(), chain: chain.into(),
        page_count: Value::Null, upload_ms: Value::Null, prepare_ms: Value::Null,
        state_polls: Value::Null, state_ready_ms: Value::Null,
        metric_download_ms: Value::Null, metric_convert_ms: Value::Null,
        first_range_ms: Value::Null, full_ms: Value::Null, e2e_ms: Value::Null,
        detail: String::new(), success: false, error: String::new(), warmup,
    };
    let mut steps: Vec<&str> = Vec::new();

    let result: Result<(), String> = async {
        // 1. 上传
        let (fid, ms) = measure(client.upload(file_path)).await;
        let fid = fid?;
        row.upload_ms = Value::from(ms as i64);
        steps.push("upload");

        // 2. prepare + state 轮询
        let (_, prep_ms) = measure(client.prepare(&fid, &biz_name)).await;
        row.prepare_ms = Value::from(prep_ms as i64);
        let t_ready = std::time::Instant::now();
        // loop-break 表达式直接产出就绪后的文件大小,避免"先初始化再覆盖"的写法
        let mut polls = 0u32;
        let file_size: u64 = loop {
            if stop.load(Ordering::Relaxed) { return Err(STOPPED.into()); }
            if polls >= cfg.poll_max_times {
                return Err(format!("state 未就绪（{} 次轮询）", polls));
            }
            polls += 1;
            let data = client.state(&fid, &biz_name, preview).await?;
            let size = data["size"].as_u64().unwrap_or(0);
            let pages = data["pageCount"].as_u64().unwrap_or(0);
            if size > 0 || pages > 0 {
                row.page_count = Value::from(pages as i64);
                if let Some(m) = data["metric"].as_object() {
                    row.metric_download_ms = m.get("download").and_then(vi64).map(Value::from).unwrap_or(Value::Null);
                    row.metric_convert_ms = m.get("convert").and_then(vi64).map(Value::from).unwrap_or(Value::Null);
                }
                break size;
            }
            tokio::time::sleep(std::time::Duration::from_millis(cfg.poll_interval_ms)).await;
        };
        row.state_polls = Value::from(polls as i64);
        row.state_ready_ms = Value::from(t_ready.elapsed().as_millis() as i64);
        steps.push("convert");

        // 3. 拉取
        if workbook {
            let (detail, full) = pull_excel(client, &fid, &biz_name, cfg.tile_batch, stop, warmup).await?;
            row.detail = detail;
            row.full_ms = Value::from(full as i64);
        } else {
            let (detail, full, first) = pull_range(client, &fid, &biz_name, file_size, cfg.range_chunk, stop, warmup).await?;
            row.detail = detail;
            row.full_ms = Value::from(full as i64);
            row.first_range_ms = Value::from(first as i64);
        }
        steps.push(chain);

        row.e2e_ms = Value::from(t_start.elapsed().as_millis() as i64);
        Ok(())
    }.await;

    row.steps = steps.iter().map(|s| s.to_string()).collect::<Vec<_>>().join(",");
    match result {
        Ok(()) => row.success = true,
        Err(e) => row.error = e,
    }
    row
}

/// 启动压测（tokio 任务池 + 抢活派发）
pub async fn start_stress(
    cfg: StressConfig,
    stop_flag: Arc<AtomicBool>,
    status: Arc<tokio::sync::Mutex<RunStatus>>,
    files: Vec<(String, String)>,
) {
    let total_tasks = files.len() * cfg.repeats as usize;
    let threads = (cfg.concurrency as usize).min(total_tasks).max(1);
    let run_id = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as i64;

    { status.lock().await.reset(); status.lock().await.log(&format!(
        "文件 {} × {} 轮 | 并发 {} | {}", files.len(), cfg.repeats, threads, cfg.steps.join(",")
    )); }

    let client = match StressClient::new(&cfg.base_url, &cfg.auth_token, &cfg.auth_headers.to_string()) {
        Ok(c) => Arc::new(c),
        Err(e) => { let mut st = status.lock().await; st.log(&format!("❌ {}", e)); st.running = false; return; }
    };
    let cfg = Arc::new(cfg);

    // 展开任务：每文件 repeats 轮，首轮标 warmup
    let tasks: Vec<(String, bool)> = files.iter()
        .flat_map(|(_, path)| (0..cfg.repeats).map(move |r| (path.clone(), r == 0)))
        .collect();
    let next = Arc::new(AtomicUsize::new(0));
    let t_wall = std::time::Instant::now();

    let workers: Vec<_> = (0..threads).map(|_| {
        let (next, tasks, cfg, client, stop_flag, status, run_id) =
            (next.clone(), tasks.clone(), cfg.clone(), client.clone(), stop_flag.clone(), status.clone(), run_id);
        tokio::spawn(async move {
            loop {
                if stop_flag.load(Ordering::Relaxed) { break; }
                let idx = next.fetch_add(1, Ordering::SeqCst);
                if idx >= tasks.len() { break; }
                let (path, warmup) = &tasks[idx];
                let row = run_task(&client, &cfg, path, idx + 1, *warmup, run_id, &stop_flag).await;

                let name = path.rsplit('/').next().unwrap_or(path);
                let icon = if row.warmup { "🔵" } else if row.success { "✅" } else { "❌" };
                let ms = |v: &Value| v.as_i64().map(|n| fmt_ms(n as u64)).unwrap_or_else(|| "-".into());
                let mut st = status.lock().await;
                st.log(&format!("{} {}#{} [{}] upload={} ready={} 全页={} {}",
                    icon, name, idx + 1, row.steps,
                    ms(&row.upload_ms), ms(&row.state_ready_ms), ms(&row.full_ms), row.detail));
                st.rows.push(row);
            }
        })
    }).collect();

    for h in workers { let _ = h.await; }

    let wall_ms = t_wall.elapsed().as_millis() as u64;
    let (success, total) = {
        let st = status.lock().await;
        let ok: Vec<_> = st.rows.iter().filter(|r| r.success).collect();
        let stable: Vec<_> = ok.iter().filter(|r| !r.warmup).collect();
        (if stable.is_empty() { ok.len() } else { stable.len() }, tasks.len())
    };
    let fpm = if wall_ms > 0 { success as f64 * 60000.0 / wall_ms as f64 } else { 0.0 };

    let mut st = status.lock().await;
    st.running = false;
    st.summary = Some(RunSummary {
        total, success, concurrency: threads as u32, wall_ms,
        files_per_min: (fpm * 100.0).round() / 100.0,
    });
    st.log(&format!("--- {}/{} | {:.1} 件/min ---", success, total, fpm));
}
