/// 时间值格式化：null → -，≥1s → x.xs，否则 xms
/// （占位符用 ASCII 连字符：等宽字体下 em dash 会触发字体回退，渲染尺寸与数字不一致）
pub fn vt(v: &serde_json::Value) -> String {
    match v.as_u64() {
        Some(ms) if ms >= 1000 => format!("{:.1}s", ms as f64 / 1000.0),
        Some(ms) => format!("{ms}ms"),
        None => "-".into(),
    }
}

pub fn vp(v: &serde_json::Value) -> String {
    v.as_u64().map(|n| n.to_string()).unwrap_or_else(|| "-".into())
}

pub fn fmt_size(n: u64) -> String {
    if n >= 1048576 { format!("{:.1}MB", n as f64 / 1048576.0) }
    else if n >= 1024 { format!("{:.1}KB", n as f64 / 1024.0) }
    else { format!("{n}B") }
}

/// 去掉 engine 侧 core-{runId}-{seq}- 前缀
pub fn strip_prefix(name: &str) -> String {
    name.strip_prefix("core-")
        .and_then(|rest| {
            let mut p = rest.splitn(3, '-');
            match (p.next(), p.next(), p.next()) {
                (Some(a), Some(b), Some(c)) if !c.is_empty()
                    && a.bytes().all(|x| x.is_ascii_digit())
                    && b.bytes().all(|x| x.is_ascii_digit()) => Some(c.to_string()),
                _ => None,
            }
        })
        .unwrap_or_else(|| name.to_string())
}
