//! xdoc-stress CLI —— 无 GUI 版本，兼容 glibc 2.28（UOS 20 / 麒麟 V10）

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use stress_core::types::{RunHandle, StressConfig};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("x-doc 核心链路压测 CLI");
        eprintln!();
        eprintln!("用法: xdoc-stress <BASE_URL> <文件...> [--concurrency N] [--repeats N]");
        eprintln!();
        eprintln!("示例:");
        eprintln!("  xdoc-stress http://host:8080/dvs /path/*.pdf");
        eprintln!("  xdoc-stress http://host:8080/dvs /path/doc.docx /path/data.xlsx --concurrency 4 --repeats 3");
        eprintln!();
        eprintln!("输出: stderr 运行日志, stdout 结果 CSV");
        std::process::exit(1);
    }

    let base_url = args[1].clone();
    let mut concurrency = 1u32;
    let mut repeats = 2u32;
    let mut files = Vec::new();

    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--concurrency" | "-c" => { i += 1; if i < args.len() { concurrency = args[i].parse().unwrap_or(1); } }
            "--repeats" | "-r" => { i += 1; if i < args.len() { repeats = args[i].parse().unwrap_or(2); } }
            _ => {
                if let Ok(p) = std::fs::canonicalize(&args[i]) {
                    files.push(p.to_string_lossy().to_string());
                }
            }
        }
        i += 1;
    }

    if files.is_empty() {
        eprintln!("❌ 没有指定有效文件");
        std::process::exit(1);
    }

    eprintln!("🎯 目标: {}", base_url);
    eprintln!("📁 文件: {} 个 × {} 轮，并发 {}", files.len(), repeats, concurrency);

    let config = StressConfig {
        base_url,
        auth_token: String::new(),
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

    let stop = Arc::new(AtomicBool::new(false));
    let handle = RunHandle::new();
    let status = handle.status.clone();

    let file_list: Vec<(String, String)> = config
        .files
        .iter()
        .map(|p| {
            let name = p.rsplit('/').next().unwrap_or(p).to_string();
            (name, p.clone())
        })
        .collect();

    let rt = tokio::runtime::Runtime::new().unwrap();

    // Ctrl-C
    let stop2 = stop.clone();
    rt.spawn(async move {
        tokio::signal::ctrl_c().await.ok();
        stop2.store(true, Ordering::Relaxed);
        eprintln!("⏹️ 停止请求");
    });

    rt.block_on(async {
        stress_core::engine::start_stress(config, stop, status, file_list).await;
    });

    // 输出 CSV 到 stdout
    let rt2 = tokio::runtime::Runtime::new().unwrap();
    rt2.block_on(async {
        let st = handle.status.lock().await;
        println!("\u{FEFF}时间戳,文件,页数,上传ms,就绪ms,下载ms,转换ms,首页ms,全页ms,端到端ms,成功,错误");
        for r in &st.rows {
            let g = |v: &serde_json::Value| v.as_i64().unwrap_or(0);
            println!(
                "{},{},{},{},{},{},{},{},{},{},{},{}",
                r.ts, r.file_name, g(&r.page_count),
                g(&r.upload_ms), g(&r.state_ready_ms),
                g(&r.metric_download_ms), g(&r.metric_convert_ms),
                g(&r.first_range_ms), g(&r.full_ms), g(&r.e2e_ms),
                r.success, r.error.replace(',', ";")
            );
        }
        if let Some(s) = &st.summary {
            eprintln!("\n━━━ {}/{} 成功 | 吞吐 {:.2} 件/min ━━━", s.success, s.total, s.files_per_min);
        }
    });
}
