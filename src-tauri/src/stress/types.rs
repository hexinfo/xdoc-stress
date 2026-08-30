use serde::{Deserialize, Serialize};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

/// 压测配置（从前端传入）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StressConfig {
    pub base_url: String,
    pub auth_token: String,
    #[serde(default)]
    pub auth_headers: serde_json::Value,
    pub steps: Vec<String>,
    pub concurrency: u32,
    pub repeats: u32,
    pub range_chunk: u64,
    pub tile_batch: usize,
    pub poll_interval_ms: u64,
    pub poll_max_times: u32,
    /// 文件绝对路径列表
    pub files: Vec<String>,
}

/// 单轮结果行（与前端表格对应）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResultRow {
    pub ts: i64,
    pub file_name: String,
    pub steps: String,
    pub chain: String,
    pub page_count: serde_json::Value,
    pub upload_ms: serde_json::Value,
    pub prepare_ms: serde_json::Value,
    pub state_polls: serde_json::Value,
    pub state_ready_ms: serde_json::Value,
    pub metric_download_ms: serde_json::Value,
    pub metric_convert_ms: serde_json::Value,
    pub first_range_ms: serde_json::Value,
    pub full_ms: serde_json::Value,
    pub e2e_ms: serde_json::Value,
    pub detail: String,
    pub success: bool,
    pub error: String,
    pub warmup: bool,
}

/// 运行摘要
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunSummary {
    pub total: usize,
    pub success: usize,
    pub concurrency: u32,
    pub wall_ms: u64,
    pub files_per_min: f64,
}

/// 运行状态快照（前端轮询）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunStatus {
    pub running: bool,
    pub stop_requested: bool,
    pub logs: Vec<String>,
    pub rows: Vec<ResultRow>,
    pub summary: Option<RunSummary>,
}

/// 共享运行句柄
pub struct RunHandle {
    pub stop_flag: Arc<AtomicBool>,
    pub status: Arc<tokio::sync::Mutex<RunStatus>>,
}

impl RunHandle {
    pub fn new() -> Self {
        Self {
            stop_flag: Arc::new(AtomicBool::new(false)),
            status: Arc::new(tokio::sync::Mutex::new(RunStatus {
                running: false,
                stop_requested: false,
                logs: Vec::new(),
                rows: Vec::new(),
                summary: None,
            })),
        }
    }
}

impl Default for RunHandle {
    fn default() -> Self {
        Self::new()
    }
}
