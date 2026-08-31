use stress_core::types::ResultRow;

use crate::formatters;

/// 左栏文件条目
pub struct FileEntry {
    pub name: String,
    pub path: String,
    pub size: String,
    pub checked: bool,
}

/// 结果表 UI 行（0 运行中 / 1 成功 / 2 失败）
#[derive(Clone)]
pub struct ResultRowUi {
    pub no: String,
    pub file_name: String,
    pub page_count: String,
    pub upload: String,
    pub ready: String,
    pub download: String,
    pub convert: String,
    pub first: String,
    pub full: String,
    pub e2e: String,
    pub state: i32,
    pub failed: bool,
}

/// ResultRow → UI 行（附带原始错误信息，供复制/导出）
pub fn to_ui_row(r: &ResultRow, seq: usize) -> (ResultRowUi, String) {
    let state = if r.warmup { 0 } else if r.success { 1 } else { 2 };
    (
        ResultRowUi {
            no: seq.to_string(),
            file_name: formatters::strip_prefix(&r.file_name),
            page_count: formatters::vp(&r.page_count),
            upload: formatters::vt(&r.upload_ms),
            ready: formatters::vt(&r.state_ready_ms),
            download: formatters::vt(&r.metric_download_ms),
            convert: formatters::vt(&r.metric_convert_ms),
            first: formatters::vt(&r.first_range_ms),
            full: formatters::vt(&r.full_ms),
            e2e: formatters::vt(&r.e2e_ms),
            state,
            failed: !r.warmup && !r.success,
        },
        r.error.clone(),
    )
}
