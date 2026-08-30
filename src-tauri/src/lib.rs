mod stress;

use std::sync::atomic::Ordering;
use std::sync::Arc;
use tauri::State;

#[derive(serde::Serialize)]
struct FileInfo {
    name: String,
    size: u64,
}

#[tauri::command]
async fn browse_dir(path: String) -> Result<BrowseResult, String> {
    let expanded = if path.starts_with('~') {
        path.replacen('~', &std::env::var("HOME").unwrap_or_default(), 1)
    } else {
        path
    };
    let entries = std::fs::read_dir(&expanded).map_err(|e| format!("读目录失败 {}: {}", expanded, e))?;
    let mut dirs = Vec::new();
    let mut files = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        if let Ok(meta) = entry.metadata() {
            if meta.is_dir() {
                dirs.push(name);
            } else if meta.is_file() {
                files.push(FileInfo { name, size: meta.len() });
            }
        }
    }
    dirs.sort();
    files.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(BrowseResult { dir: expanded, dirs, files })
}

#[derive(serde::Serialize)]
struct BrowseResult {
    dir: String,
    dirs: Vec<String>,
    files: Vec<FileInfo>,
}

#[derive(serde::Serialize)]
struct DirFile {
    name: String,
    path: String,
}

/// macos 资源fork/元数据等系统特殊文件
fn is_special_file(name: &str) -> bool {
    name.starts_with('.')
        || name == "Desktop.ini"
        || name == "Thumbs.db"
        || name.starts_with("._")
        || name.starts_with("$RECYCLE.BIN")
        || name.starts_with(".DS_")
        || name.ends_with(".tmp")
}

#[tauri::command]
async fn list_dir_files(dir_path: String) -> Result<Vec<DirFile>, String> {
    let entries = std::fs::read_dir(&dir_path).map_err(|e| format!("读目录失败 {}: {}", dir_path, e))?;
    let mut files = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if is_special_file(&name) { continue; }
        // 只收文件，忽略下级目录
        let Ok(meta) = entry.metadata() else { continue };
        if meta.is_file() {
            files.push(DirFile { name, path: entry.path().to_string_lossy().to_string() });
        }
    }
    files.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(files)
}

#[tauri::command]
async fn start_run(
    config: stress::types::StressConfig,
    run: State<'_, Arc<stress::types::RunHandle>>,
) -> Result<(), String> {
    let st = run.status.lock().await;
    if st.running {
        return Err("已有压测在运行".to_string());
    }
    drop(st);

    let mut file_list = Vec::new();
    for p in &config.files {
        let name = p.rsplit('/').next().unwrap_or(p).to_string();
        file_list.push((name, p.clone()));
    }
    if file_list.is_empty() {
        return Err("没有文件".to_string());
    }

    let stop = run.stop_flag.clone();
    let status = run.status.clone();
    stop.store(false, Ordering::Relaxed);

    tokio::spawn(async move {
        stress::engine::start_stress(config, stop, status, file_list).await;
    });
    Ok(())
}

#[tauri::command]
async fn stop_run(run: State<'_, Arc<stress::types::RunHandle>>) -> Result<(), String> {
    let st = run.status.lock().await;
    if !st.running {
        return Err("没有运行中的压测".to_string());
    }
    drop(st);
    run.stop_flag.store(true, Ordering::Relaxed);
    let mut st = run.status.lock().await;
    st.stop_requested = true;
    st.logs.push("⏹️ 收到停止请求".to_string());
    Ok(())
}

#[tauri::command]
async fn get_status(run: State<'_, Arc<stress::types::RunHandle>>) -> Result<stress::types::RunStatus, String> {
    let st = run.status.lock().await;
    eprintln!("[get_status] running={} logs={} rows={}", st.running, st.logs.len(), st.rows.len());
    Ok(st.clone())
}

#[tauri::command]
async fn export_csv(run: State<'_, Arc<stress::types::RunHandle>>) -> Result<String, String> {
    let st = run.status.lock().await;
    let mut csv = String::from("\u{FEFF}时间戳,步骤,文件名,页数,上传耗时ms,预登记耗时ms,轮询次数,就绪耗时ms,下载耗时ms,转换耗时ms,首页耗时ms,链路,明细,全页耗时ms,端到端耗时ms,成功,错误\n");
    for r in &st.rows {
        let q = |s: &str| format!("\"{}\"", s.replace('"', "\"\""));
        csv.push_str(&format!(
            "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}\n",
            r.ts,
            q(&r.steps),
            q(&r.file_name),
            r.page_count.as_i64().unwrap_or(0),
            r.upload_ms.as_i64().unwrap_or(0),
            r.prepare_ms.as_i64().unwrap_or(0),
            r.state_polls.as_i64().unwrap_or(0),
            r.state_ready_ms.as_i64().unwrap_or(0),
            r.metric_download_ms.as_i64().unwrap_or(0),
            r.metric_convert_ms.as_i64().unwrap_or(0),
            r.first_range_ms.as_i64().unwrap_or(0),
            r.chain,
            q(&r.detail),
            r.full_ms.as_i64().unwrap_or(0),
            r.e2e_ms.as_i64().unwrap_or(0),
            if r.success { "true" } else { "false" },
            q(&r.error)
        ));
    }
    Ok(csv)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(Arc::new(stress::types::RunHandle::new()))
        .invoke_handler(tauri::generate_handler![browse_dir, list_dir_files, start_run, stop_run, get_status, export_csv])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
