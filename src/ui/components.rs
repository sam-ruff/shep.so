use super::Message;
use iced::{
    Alignment, Border, Color, Element, Font, Length, Theme,
    widget::{self, button, container, row, svg, text, text_input, tooltip},
};
use std::{collections::HashMap, sync::OnceLock};

pub const BOLD: Font = Font {
    family: iced::font::Family::Name("Noto Sans"),
    weight: iced::font::Weight::Semibold,
    ..Font::DEFAULT
};
#[derive(Clone, Copy)]
pub struct Colors {
    pub bg: Color,
    pub surface: Color,
    pub subtle: Color,
    pub text: Color,
    pub muted: Color,
    pub border: Color,
    pub accent: Color,
    pub tint: Color,
    pub flag: Color,
}
pub fn colors(theme: &Theme) -> Colors {
    let dark = theme.palette().background.r < 0.3;
    if dark {
        Colors {
            bg: hex(0x141416),
            surface: hex(0x1b1b1f),
            subtle: hex(0x232329),
            text: hex(0xf4f4f5),
            muted: hex(0xa5a5b0),
            border: hex(0x323239),
            accent: hex(0xb5a0ff),
            tint: hex(0x30283f),
            flag: hex(0xf87171),
        }
    } else {
        Colors {
            bg: hex(0xf7f7f9),
            surface: Color::WHITE,
            subtle: hex(0xf4f4f7),
            text: hex(0x292830),
            muted: hex(0x777580),
            border: hex(0xe8e7ed),
            accent: hex(0x7356bd),
            tint: hex(0xf0eafa),
            flag: hex(0xc62828),
        }
    }
}
pub const fn hex(n: u32) -> Color {
    Color::from_rgb(
        ((n >> 16) & 255) as f32 / 255.,
        ((n >> 8) & 255) as f32 / 255.,
        (n & 255) as f32 / 255.,
    )
}
pub fn surface(theme: &Theme) -> container::Style {
    let p = colors(theme);
    container::Style {
        background: Some(p.surface.into()),
        text_color: Some(p.text),
        ..Default::default()
    }
}
pub fn card(theme: &Theme) -> container::Style {
    let p = colors(theme);
    container::Style {
        border: Border {
            color: p.border,
            width: 1.,
            radius: 12.into(),
        },
        ..surface(theme)
    }
}
pub fn subtle(theme: &Theme) -> container::Style {
    let p = colors(theme);
    container::Style {
        background: Some(p.subtle.into()),
        border: Border {
            radius: 8.into(),
            ..Default::default()
        },
        ..Default::default()
    }
}
pub fn line<'a>() -> Element<'a, Message> {
    container(widget::space())
        .height(1)
        .width(Length::Fill)
        .style(|t| container::Style {
            background: Some(colors(t).border.into()),
            ..Default::default()
        })
        .into()
}
pub fn muted<'a>(s: impl Into<std::borrow::Cow<'a, str>>) -> widget::Text<'a> {
    text(s.into()).size(12).style(|t| text::Style {
        color: Some(colors(t).muted),
    })
}
pub fn heading<'a>(s: impl Into<std::borrow::Cow<'a, str>>) -> widget::Text<'a> {
    text(s.into()).font(BOLD).size(23)
}
pub fn select_input(theme: &Theme, status: widget::pick_list::Status) -> widget::pick_list::Style {
    let p = colors(theme);
    widget::pick_list::Style {
        text_color: p.text,
        placeholder_color: p.muted,
        handle_color: p.muted,
        background: p.surface.into(),
        border: Border {
            color: if matches!(status, widget::pick_list::Status::Active) {
                p.border
            } else {
                p.accent
            },
            width: 1.,
            radius: 7.into(),
        },
    }
}
pub fn destructive(theme: &Theme, status: button::Status) -> button::Style {
    let mut style = primary(theme, status);
    if status != button::Status::Disabled {
        style.background = Some(
            match status {
                button::Status::Hovered | button::Status::Pressed => hex(0x991b1b),
                _ => hex(0xb91c1c),
            }
            .into(),
        );
        style.text_color = Color::WHITE;
    }
    style
}

pub fn primary(theme: &Theme, status: button::Status) -> button::Style {
    let p = colors(theme);
    if matches!(status, button::Status::Disabled) {
        return button::Style {
            background: Some(p.subtle.into()),
            text_color: p.muted,
            border: Border {
                radius: 8.into(),
                ..Default::default()
            },
            ..Default::default()
        };
    }
    let bg = match status {
        button::Status::Hovered => hex(0x8060cc),
        button::Status::Pressed => hex(0x60459f),
        _ => hex(0x7356bd),
    };
    button::Style {
        background: Some(bg.into()),
        text_color: Color::WHITE,
        border: Border {
            radius: 8.into(),
            width: if status == button::Status::Pressed {
                1.
            } else {
                0.
            },
            color: hex(0x4e3787),
        },
        ..if matches!(status, button::Status::Disabled) {
            button::Style {
                background: Some(p.subtle.into()),
                text_color: p.muted,
                ..Default::default()
            }
        } else {
            Default::default()
        }
    }
}
pub fn ghost(theme: &Theme, status: button::Status) -> button::Style {
    let p = colors(theme);
    button::Style {
        background: if status == button::Status::Pressed {
            Some(p.tint.into())
        } else if status == button::Status::Hovered {
            Some(p.subtle.into())
        } else {
            None
        },
        text_color: if matches!(status, button::Status::Disabled) {
            p.muted
        } else {
            p.text
        },
        border: Border {
            radius: 7.into(),
            ..Default::default()
        },
        ..Default::default()
    }
}
pub fn outline(theme: &Theme, status: button::Status) -> button::Style {
    let p = colors(theme);
    button::Style {
        border: Border {
            color: p.border,
            width: 1.,
            radius: 8.into(),
        },
        ..ghost(theme, status)
    }
}
pub fn selected(theme: &Theme, _: button::Status) -> button::Style {
    let p = colors(theme);
    button::Style {
        background: Some(p.tint.into()),
        text_color: p.accent,
        border: Border {
            radius: 8.into(),
            ..Default::default()
        },
        ..Default::default()
    }
}
pub fn field(theme: &Theme, status: text_input::Status) -> text_input::Style {
    let p = colors(theme);
    text_input::Style {
        background: p.surface.into(),
        border: Border {
            color: if matches!(status, text_input::Status::Focused { .. }) {
                p.accent
            } else {
                p.border
            },
            width: 1.,
            radius: 8.into(),
        },
        icon: p.muted,
        placeholder: p.muted,
        value: p.text,
        selection: p.tint,
    }
}
pub fn input<'a>(
    placeholder: &str,
    value: &str,
    on_input: impl Fn(String) -> Message + 'a,
) -> widget::TextInput<'a, Message> {
    text_input(placeholder, value)
        .on_input(on_input)
        .size(13)
        .padding([11, 12])
        .style(field)
}
pub fn action<'a>(label: &'a str, message: Message) -> widget::Button<'a, Message> {
    button(text(label).size(13))
        .padding([10, 14])
        .style(outline)
        .on_press(message)
}
impl super::App {
    pub(super) fn icon_action<'a>(
        &self,
        name: &str,
        label: impl Into<std::borrow::Cow<'a, str>>,
        message: Message,
    ) -> Element<'a, Message> {
        self.toggle_icon_action(name, label, false, message)
    }
    pub(super) fn toggle_icon_action<'a>(
        &self,
        name: &str,
        label: impl Into<std::borrow::Cow<'a, str>>,
        active: bool,
        message: Message,
    ) -> Element<'a, Message> {
        let control = button(if name == "flag" {
            flag_icon(active, 20.)
        } else {
            icon(name, 20.)
        })
        .padding(10)
        .style(if active && name == "flag" {
            flagged
        } else if active {
            selected
        } else {
            ghost
        })
        .on_press(message);
        if !self.preferences.tooltips {
            return control.into();
        }
        tooltip(
            control,
            container(text(label.into()).size(12))
                .padding(8)
                .style(card),
            tooltip::Position::Bottom,
        )
        .gap(5)
        .into()
    }
}
pub fn badge<'a>(label: impl Into<std::borrow::Cow<'a, str>>) -> Element<'a, Message> {
    container(text(label.into()).size(10).font(BOLD))
        .padding([4, 8])
        .style(|t| {
            let p = colors(t);
            container::Style {
                background: Some(p.tint.into()),
                text_color: Some(p.accent),
                border: Border {
                    radius: 5.into(),
                    ..Default::default()
                },
                ..Default::default()
            }
        })
        .into()
}
pub fn nav<'a>(
    name: &str,
    label: &'a str,
    active: bool,
    message: Message,
    count: Option<usize>,
) -> Element<'a, Message> {
    let mut content = row![
        icon(name, 20.),
        text(label).size(13).line_height(1.),
        widget::space().width(Length::Fill)
    ]
    .spacing(11)
    .align_y(Alignment::Center);
    if let Some(count) = count {
        content = content.push(text(count.to_string()).size(11));
    }
    button(content)
        .padding([11, 12])
        .width(Length::Fill)
        .style(if active { selected } else { ghost })
        .on_press(message)
        .into()
}
pub fn avatar<'a>(name: &str, index: usize, size: f32) -> Element<'a, Message> {
    let words: Vec<_> = name.split_whitespace().collect();
    let initials = words
        .iter()
        .take(2)
        .filter_map(|w| w.chars().next())
        .collect::<String>()
        .to_uppercase();
    let palettes = [
        (0xeee5e2, 0x9b6960),
        (0xe5eaf5, 0x5976a1),
        (0xe8eadd, 0x778454),
        (0xf1e7f4, 0x9a69a7),
        (0xf5eddc, 0xa18b52),
    ];
    let (bg, fg) = palettes[index % palettes.len()];
    container(text(initials).size(size * 0.36).font(BOLD).color(hex(fg)))
        .center_x(size)
        .center_y(size)
        .style(move |_| container::Style {
            background: Some(hex(bg).into()),
            border: Border {
                radius: (size / 2.).into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
}
pub fn flagged(theme: &Theme, status: button::Status) -> button::Style {
    let mut style = ghost(theme, status);
    style.border = Border {
        color: colors(theme).flag,
        width: 1.5,
        radius: 7.into(),
    };
    style.text_color = colors(theme).flag;
    style
}
pub fn flag_icon<'a>(active: bool, size: f32) -> Element<'a, Message> {
    icon_color("flag", size, false, active)
}
pub fn icon<'a>(name: &str, size: f32) -> Element<'a, Message> {
    icon_color(name, size, false, false)
}
pub fn icon_bright<'a>(name: &str, size: f32) -> Element<'a, Message> {
    icon_color(name, size, true, false)
}
fn icon_color<'a>(name: &str, size: f32, bright: bool, is_flagged: bool) -> Element<'a, Message> {
    static ICONS: OnceLock<HashMap<&'static str, svg::Handle>> = OnceLock::new();
    let icons=ICONS.get_or_init(||[
        ("image",r#"<rect x="3" y="3" width="18" height="18" rx="2"/><circle cx="8" cy="8" r="2"/><path d="m21 15-5-5L5 21"/>"#),
        ("mail",r#"<rect x="3" y="5" width="18" height="14" rx="2"/><path d="m3 6 9 7 9-7"/>"#),
        ("inbox",r#"<path d="M4 4h16l2 11v5H2v-5L4 4Z"/><path d="M2 15h6l2 3h4l2-3h6"/>"#),
        ("calendar",r#"<rect x="3" y="5" width="18" height="16" rx="2"/><path d="M16 3v4M8 3v4M3 11h18M8 15h2M14 15h2"/>"#),
        ("compose",r#"<path d="m15 4 5 5M4 20l4-1L21 6a2 2 0 0 0-3-3L5 16l-1 4ZM13 4H5a2 2 0 0 0-2 2v14a1 1 0 0 0 1 1h14a2 2 0 0 0 2-2v-6"/>"#),
        ("flag",r#"<path d="M5 21V4c5-4 9 4 14 0v11c-5 4-9-4-14 0"/>"#),
        ("star",r#"<path d="m12 3 2.8 5.8 6.4.9-4.6 4.5 1.1 6.3-5.7-3-5.7 3 1.1-6.3-4.6-4.5 6.4-.9L12 3Z"/>"#),
        ("send",r#"<path d="m22 2-7 20-4-9-9-4L22 2ZM22 2 11 13"/>"#),
        ("file",r#"<path d="M14 2H5v20h14V7l-5-5ZM14 2v6h5M8 13h8M8 17h6"/>"#),
        ("archive",r#"<rect x="3" y="3" width="18" height="4" rx="1"/><path d="M5 7v14h14V7M10 11h4"/>"#),
        ("trash",r#"<path d="M3 6h18M9 6V3h6v3M5 6l1 15h12l1-15M10 10v7M14 10v7"/>"#),
        ("folder",r#"<path d="M3 7V4h6l2 3h10v13H3V7Z"/>"#),
        ("move",r#"<path d="M3 7V4h6l2 3h10v13H3V7ZM8 14h8m-3-3 3 3-3 3"/>"#),
        ("plus",r#"<path d="M12 5v14M5 12h14"/>"#),
        ("search",r#"<circle cx="10.5" cy="10.5" r="6.5"/><path d="m16 16 5 5"/>"#),
        ("down",r#"<path d="m6 9 6 6 6-6"/>"#),
        ("chevron-down",r#"<path d="m5 9 7 7 7-7"/>"#),
        ("chevron",r#"<path d="m9 5 7 7-7 7"/>"#),
        ("left",r#"<path d="m15 5-7 7 7 7"/>"#),
        ("down",r#"<path d="m6 9 6 6 6-6"/>"#),
        ("sync",r#"<path d="M20 7v5h-5M4 17v-5h5M5 7a8 8 0 0 1 13-2l2 3M4 16l2 3a8 8 0 0 0 13-2"/>"#),
        ("settings",r#"<path d="m9 3 1-1h4l1 3 3 1 3 3-1 3 1 3-3 3-3 1-1 3h-4l-1-3-3-1-3-3 1-3-1-3 3-3 3-1V3Z"/><circle cx="12" cy="12" r="3"/>"#),
        ("shield",r#"<path d="m12 2 9 4v6c0 5-9 10-9 10S3 17 3 12V6l9-4Z"/><path d="m8 12 3 3 5-6"/>"#),
        ("cloud",r#"<path d="M6 18a5 5 0 0 1-1-10 7 7 0 0 1 13-2 6 6 0 0 1 0 12H6Z"/>"#),
        ("reply",r#"<path d="m9 4-6 6 6 6M3 10h11a7 7 0 0 1 7 7v3"/>"#),
        ("clip",r#"<path d="m21 11-9 9a6 6 0 0 1-8-8L14 2a4 4 0 0 1 6 6L10 18a2 2 0 0 1-3-3l9-9"/>"#),
        ("close",r#"<path d="m6 6 12 12M6 18 18 6"/>"#),
        ("check",r#"<path d="m5 12 4 4L19 6"/>"#),
        ("sun",r#"<circle cx="12" cy="12" r="4"/><path d="M12 2v2M12 20v2M2 12h2M20 12h2M5 5l1 1M18 18l1 1M5 19l1-1M18 6l1-1"/>"#),
        ("moon",r#"<path d="M21 13A9 9 0 0 1 11 3 9 9 0 1 0 21 13Z"/>"#),
        ("keyboard",r#"<rect x="2" y="5" width="20" height="14" rx="2"/><path d="M6 9h1m3 0h1m3 0h1m3 0h1M6 12h1m3 0h1m3 0h1m3 0h1M7 16h10"/>"#),
        ("copy",r#"<rect x="8" y="8" width="12" height="13" rx="2"/><path d="M16 8V5a2 2 0 0 0-2-2H5a2 2 0 0 0-2 2v10a2 2 0 0 0 2 2h3"/>"#),
        ("download",r#"<path d="M12 3v12m-5-5 5 5 5-5M4 16v5h16v-5"/>"#),
        ("clock",r#"<circle cx="12" cy="12" r="9"/><path d="M12 7v5l3 2"/>"#),
    ].into_iter().map(|(name,path)|(name,svg::Handle::from_memory(format!(r##"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="#777580" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">{path}</svg>"##).into_bytes()))).collect());
    svg(icons.get(name).unwrap_or(&icons["mail"]).clone())
        .width(size)
        .height(size)
        .style(move |t, _| svg::Style {
            color: Some(if is_flagged {
                colors(t).flag
            } else if bright {
                Color::WHITE
            } else {
                colors(t).muted
            }),
        })
        .into()
}

pub fn editor_field(
    theme: &Theme,
    status: widget::text_editor::Status,
) -> widget::text_editor::Style {
    let p = colors(theme);
    widget::text_editor::Style {
        background: p.surface.into(),
        border: Border {
            radius: 8.into(),
            width: 1.,
            color: if matches!(status, widget::text_editor::Status::Focused { .. }) {
                p.accent
            } else {
                p.border
            },
        },
        placeholder: p.muted,
        value: p.text,
        selection: p.tint,
    }
}

pub fn select_menu(theme: &Theme) -> widget::overlay::menu::Style {
    let p = colors(theme);
    widget::overlay::menu::Style {
        background: p.surface.into(),
        border: Border {
            radius: 8.into(),
            width: 1.,
            color: p.border,
        },
        text_color: p.text,
        selected_text_color: p.accent,
        selected_background: p.tint.into(),
        shadow: iced::Shadow::default(),
    }
}
