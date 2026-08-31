use iced::Color;

const fn c(r: u8, g: u8, b: u8) -> Color {
    Color::from_rgb8(r, g, b)
}

/// 窗口/根背景
pub const BG: Color = c(0xf0, 0xf2, 0xf5);
/// 主文字
pub const INK: Color = c(0x1a, 0x1d, 0x24);
/// 次级文字
pub const INK2: Color = c(0x33, 0x38, 0x3f);
/// 字段标签/状态文字
pub const GRAY: Color = c(0x5f, 0x66, 0x72);
/// 日志文字
pub const LOG: Color = c(0x42, 0x4a, 0x53);
/// 空态提示
pub const HINT: Color = c(0x9b, 0xa1, 0xad);
/// 主题蓝（小节标题/主按钮/运行中圆点）
pub const BLUE: Color = c(0x25, 0x63, 0xeb);
pub const BLUE_HOVER: Color = c(0x1d, 0x4e, 0xd8);
pub const BLUE_LIGHT_BG: Color = c(0xf0, 0xf4, 0xff);
/// 危险红（停止/失败）
pub const RED: Color = c(0xdc, 0x26, 0x26);
pub const RED_BORDER: Color = c(0xfc, 0xa5, 0xa5);
pub const RED_LIGHT_BG: Color = c(0xfe, 0xf2, 0xf2);
/// 成功绿
pub const GREEN: Color = c(0x16, 0xa3, 0x4a);
/// 端到端数值（琥珀）
pub const AMBER: Color = c(0xb4, 0x53, 0x09);
/// 白
pub const WHITE: Color = Color::WHITE;
/// 输入框边框
pub const BORDER: Color = c(0xd9, 0xdc, 0xe1);
/// 面板边框
pub const BORDER_PANEL: Color = c(0xe3, 0xe6, 0xea);
/// 表格/日志内框
pub const BORDER_FAINT: Color = c(0xee, 0xf0, 0xf3);
/// 禁用
pub const DISABLED_BG: Color = c(0xf5, 0xf6, 0xf8);
pub const DISABLED_FG: Color = c(0xc3, 0xc8, 0xd0);
/// 表头/日志底色
pub const HEADER_BG: Color = c(0xf8, 0xf9, 0xfb);
/// 结果行 hover
pub const ROW_HOVER: Color = c(0xf8, 0xfa, 0xff);
