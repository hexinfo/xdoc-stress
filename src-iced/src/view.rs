//! 三栏视图。
//!
//! 布局红线(iced 0.14 + cosmic-text 0.15.0 的坑,违反会 panic):
//! 1. 等宽字体 + CJK/特殊符号混排会触发 cosmic-text 字形回退 inf 坐标 panic,
//!    且等宽与默认字体同字号渲染不一致——全应用统一默认字体。
//! 2. 空值占位符用 ASCII "-",em dash 在回退字体下渲染偏大。
//! 3. 结果表禁用 Grid:其均分布局在无界空间(双向滚动)会以 inf 计算单元格位置,
//!    tiny-skia 画背景四边形时 panic("Build quad rectangle")。改用全定宽行布局。
//! 4. 纵向滚动内容须显式 `width(Fill)` 断界。

use iced::font::Weight;
use iced::font::Family;
use iced::widget::text::Wrapping;
use iced::widget::text_input::TextInput;
use iced::widget::scrollable::{Direction, Scrollbar};
use iced::widget::{Space, button, checkbox, column, container, lazy, mouse_area, row, scrollable, text, text_input};
use iced::widget::{button::Button, container::Container};
use iced::{Color, Element, Font, Length};
use iced::alignment::Horizontal;
use iced::alignment::Vertical;

use crate::model::{FileEntry, ResultRowUi};
use crate::palette as p;
use crate::styles::{self, BtnKind};
use crate::{App, Message};

/// 文件名列宽度:按字符数估算(ASCII 7px / 非 ASCII 12px,宁宽勿窄),保证单行不裁字
fn est_file_col_width<'a>(names: impl Iterator<Item = &'a String>) -> f32 {
    let mut max = 0.0f32;
    for name in names {
        let w: f32 = name.chars().map(|c| if c.is_ascii() { 9.5 } else { 14.0 }).sum();
        max = max.max(w);
    }
    (max + 20.0).max(160.0)
}

pub fn view(app: &App) -> Element<'_, Message> {
    container(
        row![files_panel(app), main_panel(app), config_panel(app)].spacing(8),
    )
    .padding(8)
    .width(Length::Fill)
    .height(Length::Fill)
    .style(|_| styles::root())
    .into()
}

// ════ 左栏:文件 ════

fn files_panel(app: &App) -> Element<'_, Message> {
    let header = row![
        title("文件"),
        Space::new().width(Length::Fill),
        checkbox(app.all_checked).label("全选").text_size(12).on_toggle(Message::ToggleAll),
        small_btn("清空", Some(Message::ClearFiles), BtnKind::Plain),
    ]
    .spacing(6)
    .align_y(Vertical::Center);

    let actions = row![
        normal_btn("文件夹", Some(Message::PickFolder), BtnKind::Plain).width(Length::Fill),
        normal_btn("文件", Some(Message::PickFiles), BtnKind::Plain).width(Length::Fill),
    ]
    .spacing(6);

    let list: Element<'_, Message> = if app.files.is_empty() {
        container(
            text("点上方按钮选择文件夹或文件")
                .size(12)
                .color(p::HINT),
        )
        .center(Length::Fill)
        .into()
    } else {
        let rows: Vec<Element<'_, Message>> = app
            .files
            .iter()
            .enumerate()
            .map(|(i, f)| file_row(i, f, app.hovered_file_row == Some(i)))
            .collect();
        scrollable(column(rows).padding(2).width(Length::Fill))
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    };

    container(
        column![
            header,
            actions,
            container(list).width(Length::Fill).height(Length::Fill),
            text(format!("{} 个文件", app.files.len()))
                .size(12)
                .color(p::HINT)
                .height(Length::Fixed(18.0)),
        ]
        .spacing(6)
        .padding(8)
        .width(Length::Fill)
        .height(Length::Fill),
    )
    .width(Length::Fixed(300.0))
    .height(Length::Fill)
    .style(|_| styles::panel())
    .into()
}

/// 文件行:单行不换行,超宽裁剪
fn file_row(i: usize, f: &FileEntry, hovered: bool) -> Element<'_, Message> {
    let bg = if hovered { Some(p::BLUE_LIGHT_BG) } else { None };
    mouse_area(
        container(
            row![
                checkbox(f.checked).on_toggle(move |b| Message::ToggleFile(i, b)),
                // 名字区可点击:单击切换勾选,Shift+单击范围多选(复选框区不受影响)
                mouse_area(
                    container(
                        text(f.name.as_str()).size(12).color(p::INK2).wrapping(Wrapping::None),
                    )
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .clip(true),
                )
                .on_press(Message::RowClick(i)),
            ]
            .spacing(5)
            .padding([0.0, 5.0]),
        )
        .width(Length::Fill)
        .height(Length::Fixed(22.0))
        .align_y(Vertical::Center)
        .clip(true)
        .style(move |_| styles::row_bg(bg, 4.0)),
    )
    .on_enter(Message::FileRowHover(Some(i)))
    .on_exit(Message::FileRowHover(None))
    .interaction(iced::mouse::Interaction::Pointer)
    .into()
}

// ════ 中栏:结果 + 日志 ════

fn main_panel(app: &App) -> Element<'_, Message> {
    column![
        container(result_panel(app)).width(Length::Fill).height(Length::FillPortion(3)),
        container(log_panel(app)).width(Length::Fill).height(Length::FillPortion(1)),
    ]
    .spacing(8)
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

fn result_panel(app: &App) -> Element<'_, Message> {
    let header = row![
        title("结果"),
        // 状态文字与标题同字号,仅以灰度区分层级
        text(app.status_text.as_str()).size(14).color(p::GRAY).width(Length::Fill),
        small_btn("复制", Some(Message::CopyResults), BtnKind::Plain),
        small_btn("CSV", Some(Message::ExportCsv), BtnKind::Plain),
    ]
    .spacing(6)
    .align_y(Vertical::Center);

    let table: Element<'_, Message> = if app.rows.is_empty() {
        container(
            text(if app.running { "运行中…" } else { "配置参数后点「开始测试」" })
                .size(12)
                .color(p::HINT),
        )
        .center(Length::Fill)
        .into()
    } else {
        // 全部单元格定宽(文件名列按内容估算),行宽一致 → 各列天然对齐;
        // 外层双向 scrollable 提供横向滚动。
        let file_w = est_file_col_width(app.rows.iter().map(|(r, _)| &r.file_name));

        // lazy:仅在行数或悬停行变化时重建控件树;
        // 轮询 tick/输入/滚动触发的重绘直接复用缓存树(滚动流畅的关键)。
        // 行数据只在追加时变化(引擎每任务 push 一次),行数足以代表内容变化。
        let rows = app.rows.clone();
        let hovered = app.hovered_result_row;
        scrollable(
            lazy((rows.len(), hovered), move |_| {
                let mut items: Vec<Element<'static, Message>> = vec![table_header(file_w).into()];
                items.extend(
                    rows.iter()
                        .enumerate()
                        .map(|(i, (r, _))| table_row(i, r, file_w, hovered == Some(i))),
                );
                column(items)
            }),
        )
        .direction(Direction::Both {
            vertical: Scrollbar::new(),
            horizontal: Scrollbar::new(),
        })
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    };

    container(
        column![
            header,
            container(table)
                .width(Length::Fill)
                .height(Length::Fill)
                .clip(true)
                .style(|_| styles::table_frame()),
        ]
        .spacing(6)
        .padding(8)
        .width(Length::Fill)
        .height(Length::Fill),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .style(|_| styles::panel())
    .into()
}

/// 表头行(10px 600 灰)
fn table_header(file_w: f32) -> Container<'static, Message> {
    container(
        row![
            hcell("#", Length::Fixed(22.0), Horizontal::Left),
            hcell("文件", Length::Fixed(file_w), Horizontal::Left),
            hcell("页数", Length::Fixed(30.0), Horizontal::Right),
            hcell("上传", Length::Fixed(44.0), Horizontal::Right),
            hcell("就绪", Length::Fixed(44.0), Horizontal::Right),
            hcell("下载", Length::Fixed(44.0), Horizontal::Right),
            hcell("转换", Length::Fixed(44.0), Horizontal::Right),
            hcell("首页", Length::Fixed(44.0), Horizontal::Right),
            hcell("全页", Length::Fixed(44.0), Horizontal::Right),
            hcell("端到端", Length::Fixed(48.0), Horizontal::Right),
            hcell("状态", Length::Fixed(26.0), Horizontal::Center),
        ]
        .spacing(6)
        .padding([0.0, 8.0]),
    )
    .height(Length::Fixed(26.0))
    .align_y(Vertical::Center)
    .style(|_| styles::header_bg())
}

fn table_row(i: usize, r: &ResultRowUi, file_w: f32, hovered: bool) -> Element<'static, Message> {
    let bg = row_bg_of(r.failed, hovered);
    mouse_area(
        container(
            row![
                cell(r.no.clone(), Length::Fixed(22.0), Horizontal::Left, p::HINT),
                cell(r.file_name.clone(), Length::Fixed(file_w), Horizontal::Left, p::INK2),
                cell(r.page_count.clone(), Length::Fixed(30.0), Horizontal::Right, p::INK2),
                cell(r.upload.clone(), Length::Fixed(44.0), Horizontal::Right, p::INK2),
                cell(r.ready.clone(), Length::Fixed(44.0), Horizontal::Right, p::INK2),
                cell(r.download.clone(), Length::Fixed(44.0), Horizontal::Right, p::INK2),
                cell(r.convert.clone(), Length::Fixed(44.0), Horizontal::Right, p::INK2),
                cell(r.first.clone(), Length::Fixed(44.0), Horizontal::Right, p::INK2),
                cell(r.full.clone(), Length::Fixed(44.0), Horizontal::Right, p::INK2),
                cell(r.e2e.clone(), Length::Fixed(48.0), Horizontal::Right, p::AMBER),
                dot_cell(r.state, bg),
            ]
            .spacing(6)
            .padding([0.0, 8.0]),
        )
        .width(Length::Shrink)
        .height(Length::Fixed(24.0))
        .style(move |_| styles::row_bg(bg, 0.0)),
    )
    .on_enter(Message::ResultRowHover(Some(i)))
    .on_exit(Message::ResultRowHover(None))
    .into()
}

/// 表头单元格
fn hcell<'a>(s: &'a str, width: Length, h: Horizontal) -> Container<'a, Message> {
    let t = text(s)
        .size(12)
        .color(p::GRAY)
        .font(Font { family: Family::SansSerif, weight: Weight::Semibold, ..Font::DEFAULT })
        .wrapping(Wrapping::None);
    container(t)
        .width(width)
        .height(Length::Fill)
        .align_y(Vertical::Center)
        .align_x(h)
        .clip(true)
}

/// 数据单元格(11px,单行定宽;持有所有权字符串以满足 Lazy 的 'static 约束)
fn cell(s: String, width: Length, h: Horizontal, color: Color) -> Container<'static, Message> {
    let t = text(s).size(12).color(color).wrapping(Wrapping::None);
    container(t)
        .width(width)
        .height(Length::Fill)
        .align_y(Vertical::Center)
        .align_x(h)
        .clip(true)
}

/// 状态圆点单元格(8px,0 蓝 / 1 绿 / 2 红)
fn dot_cell<'a>(state: i32, bg: Option<Color>) -> Container<'a, Message> {
    let color = match state {
        0 => p::BLUE,
        1 => p::GREEN,
        _ => p::RED,
    };
    container(
        container(Space::new())
            .width(8)
            .height(8)
            .style(move |_| styles::dot(color)),
    )
    .width(Length::Fixed(26.0))
    .height(Length::Fill)
    .align_x(Horizontal::Center)
    .align_y(Vertical::Center)
    .style(move |_| styles::row_bg(bg, 0.0))
}

fn row_bg_of(failed: bool, hovered: bool) -> Option<Color> {
    if failed {
        Some(p::RED_LIGHT_BG)
    } else if hovered {
        Some(p::ROW_HOVER)
    } else {
        None
    }
}

fn log_panel(app: &App) -> Element<'_, Message> {
    let body = if app.log_text.is_empty() {
        text("等待运行…").size(12).color(p::LOG)
    } else {
        text(app.log_text.as_str()).size(12).color(p::LOG)
    };

    container(
        column![
            title("运行日志"),
            container(
                scrollable(container(body).padding(6).width(Length::Fill))
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .width(Length::Fill)
            .height(Length::Fill)
            .style(|_| styles::log_frame()),
        ]
        .spacing(6)
        .padding(8)
        .width(Length::Fill)
        .height(Length::Fill),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .style(|_| styles::panel())
    .into()
}

// ════ 右栏:配置 ════

fn config_panel(app: &App) -> Element<'_, Message> {
    let cfg = &app.cfg;
    container(
        column![
            title("配置"),
            scrollable(
                column![
                    section("目标"),
                    glabel("BASE_URL"),
                    field("http://host:8080/dvs", &cfg.base_url, Message::BaseUrlChanged),
                    glabel("AUTH_TOKEN（Bearer）"),
                    field("eyJhbGciOi…（留空则不带）", &cfg.auth_token, Message::AuthTokenChanged),
                    glabel("AUTH_HEADERS（JSON，Cookie 等）"),
                    field("{\"Cookie\":\"JSESSIONID=abc123\"}", &cfg.auth_headers, Message::AuthHeadersChanged),
                    section("并发"),
                    row![
                        field_col("线程", &cfg.concurrency, Message::ConcurrencyChanged),
                        field_col("轮次", &cfg.repeats, Message::RepeatsChanged),
                    ].spacing(6),
                    section("拉取"),
                    row![
                        field_col("分段 B", &cfg.range_chunk, Message::RangeChunkChanged),
                        field_col("瓦片批", &cfg.tile_batch, Message::TileBatchChanged),
                    ].spacing(6),
                    section("轮询"),
                    row![
                        field_col("间隔 ms", &cfg.poll_interval, Message::PollIntervalChanged),
                        field_col("上限", &cfg.poll_max, Message::PollMaxChanged),
                    ].spacing(6),
                    Space::new().height(Length::Fixed(4.0)),
                    row![
                        normal_btn("▶ 开始测试", if app.running { None } else { Some(Message::StartTest) }, BtnKind::Primary)
                            .width(Length::Fill),
                        normal_btn("■ 停止", if app.running { Some(Message::StopTest) } else { None }, BtnKind::Danger)
                            .width(Length::Fill),
                    ].spacing(6),
                ]
                .spacing(5)
                .width(Length::Fill),
            )
            .width(Length::Fill)
            .height(Length::Fill),
        ]
        .spacing(6)
        .padding(10)
        .width(Length::Fill)
        .height(Length::Fill),
    )
    .width(Length::Fixed(300.0))
    .height(Length::Fill)
    .style(|_| styles::panel())
    .into()
}

// ════ 小组件 ════

/// 面板标题(13px 粗)
fn title<'a>(s: &'a str) -> iced::widget::Text<'a> {
    // 600(Semibold)是苹方的真实字重;700 会触发仿粗体合成,小字号中文笔画粘连
    text(s).size(14).color(p::INK).font(Font { family: Family::SansSerif, weight: Weight::Semibold, ..Font::DEFAULT })
}

/// 蓝色小节标题(11px 粗蓝)
fn section<'a>(s: &'a str) -> iced::widget::Text<'a> {
    text(s).size(12).color(p::BLUE).font(Font { family: Family::SansSerif, weight: Weight::Semibold, ..Font::DEFAULT })
}

/// 灰色字段标签(11px)
fn glabel<'a>(s: &'a str) -> iced::widget::Text<'a> {
    text(s).size(12).color(p::GRAY)
}

/// 单行输入(白底、placeholder;不可用等宽字体,见文件头注释)
fn field<'a>(placeholder: &'a str, value: &'a str, on: fn(String) -> Message) -> TextInput<'a, Message> {
    text_input(placeholder, value)
        .on_input(on)
        .size(12)
        .padding([4.0, 8.0])
        .width(Length::Fill)
}

/// 标签 + 输入的两行小节(横向各占一半)
fn field_col<'a>(label: &'a str, value: &'a str, on: fn(String) -> Message) -> Element<'a, Message> {
    column![glabel(label), field("", value, on)]
        .spacing(3)
        .width(Length::Fill)
        .into()
}

/// 按钮(small 22px / 普通 32px;on_press 为 None 时呈禁用态)
/// 文字统一 11px Semibold,与全局正文字号一致,不随按钮尺寸变化
fn btn<'a>(label: &'a str, on: Option<Message>, kind: BtnKind, small: bool) -> Button<'a, Message> {
    let body = container(
        text(label).size(12).font(Font { family: Family::SansSerif, weight: Weight::Semibold, ..Font::DEFAULT }),
    )
    .center(Length::Fill);
    let mut b = button(body)
        .padding(if small { [2.0, 8.0] } else { [6.0, 8.0] })
        .height(Length::Fixed(if small { 22.0 } else { 32.0 }))
        .style(move |_, status| styles::btn(kind, small, status));
    if let Some(msg) = on {
        b = b.on_press(msg);
    }
    b
}

fn small_btn<'a>(label: &'a str, on: Option<Message>, kind: BtnKind) -> Button<'a, Message> {
    btn(label, on, kind, true)
}

fn normal_btn<'a>(label: &'a str, on: Option<Message>, kind: BtnKind) -> Button<'a, Message> {
    btn(label, on, kind, false)
}
