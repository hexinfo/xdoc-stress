use iced::{Border, Color, Shadow};
use iced::widget::{button, container};

use crate::palette as p;

/// 按钮种类（对应 Slint 版 Btn 的 primary/danger/普通三态）
#[derive(Clone, Copy, PartialEq)]
pub enum BtnKind {
    Primary,
    Danger,
    Plain,
}

/// 按钮样式（禁用/hover 配色与 Slint 版逐项对齐）
pub fn btn(kind: BtnKind, small: bool, status: button::Status) -> button::Style {
    let enabled = !matches!(status, button::Status::Disabled);
    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);

    let (bg, border, fg) = if !enabled {
        (p::DISABLED_BG, p::BORDER_PANEL, p::DISABLED_FG)
    } else {
        match kind {
            BtnKind::Primary => (
                if hovered { p::BLUE_HOVER } else { p::BLUE },
                p::BLUE,
                p::WHITE,
            ),
            BtnKind::Danger => (
                if hovered { p::RED_LIGHT_BG } else { p::WHITE },
                p::RED_BORDER,
                p::RED,
            ),
            BtnKind::Plain => (
                if hovered { p::BLUE_LIGHT_BG } else { p::WHITE },
                p::BORDER,
                p::INK2,
            ),
        }
    };

    button::Style {
        background: Some(bg.into()),
        text_color: fg,
        border: Border {
            color: border,
            width: 1.0,
            radius: (if small { 5.0 } else { 7.0 }).into(),
        },
        shadow: Shadow::default(),
        ..button::Style::default()
    }
}

/// 根容器（窗口灰底）
pub fn root() -> container::Style {
    container::Style {
        background: Some(p::BG.into()),
        ..container::Style::default()
    }
}

/// 白底圆角面板卡片
pub fn panel() -> container::Style {
    container::Style {
        background: Some(p::WHITE.into()),
        border: Border { color: p::BORDER_PANEL, width: 1.0, radius: 10.0.into() },
        ..container::Style::default()
    }
}

/// 结果表外框
pub fn table_frame() -> container::Style {
    container::Style {
        border: Border { color: p::BORDER_FAINT, width: 1.0, radius: 6.0.into() },
        ..container::Style::default()
    }
}

/// 日志区（浅灰底）
pub fn log_frame() -> container::Style {
    container::Style {
        background: Some(p::HEADER_BG.into()),
        border: Border { color: p::BORDER_FAINT, width: 1.0, radius: 6.0.into() },
        ..container::Style::default()
    }
}

/// 表头行
pub fn header_bg() -> container::Style {
    container::Style {
        background: Some(p::HEADER_BG.into()),
        ..container::Style::default()
    }
}

/// 行底色（None → 透明）
pub fn row_bg(color: Option<Color>, radius: f32) -> container::Style {
    container::Style {
        background: color.map(Color::into),
        border: Border { color: Color::TRANSPARENT, width: 0.0, radius: radius.into() },
        ..container::Style::default()
    }
}

/// 状态圆点
pub fn dot(color: Color) -> container::Style {
    container::Style {
        background: Some(color.into()),
        border: Border { color, width: 0.0, radius: 4.0.into() },
        ..container::Style::default()
    }
}
