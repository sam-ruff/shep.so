use super::*;
use crate::appearance::{Palette, Palettes, Rgb, Role};
use iced::theme::palette::{Extended, Pair};
use iced::widget::{button, column, container, pick_list, row, space, text};
use iced::{Alignment, Border, Length};
use std::cell::RefCell;

/// Owned by the UI thread; only rebuild the iced palette when its input changes.
#[derive(Default)]
pub(super) struct ThemeCache(RefCell<Option<(Palette, bool, Theme)>>);
impl ThemeCache {
    pub fn get(&self, palette: Palette, dark: bool) -> Theme {
        let mut cached = self.0.borrow_mut();
        if let Some((previous, previous_dark, theme)) = &*cached
            && *previous == palette
            && *previous_dark == dark
        {
            return theme.clone();
        }
        let theme = make_theme(palette, dark);
        *cached = Some((palette, dark, theme.clone()));
        theme
    }
}
pub(super) fn make_theme(p: Palette, dark: bool) -> Theme {
    let rgb = |value: Rgb| hex(value.value());
    Theme::custom_with_fn(
        if dark { "Shep Dark" } else { "Shep Light" },
        iced::theme::Palette {
            background: rgb(p.background),
            text: rgb(p.text),
            primary: rgb(p.accent),
            success: rgb(p.success),
            warning: rgb(p.warning),
            danger: rgb(p.danger),
        },
        move |base| {
            let mut extended = Extended::generate(base);
            // Semantic pairs are shared by iced's stock widgets and Shep components.
            extended.is_dark = dark;
            extended.background.base = Pair {
                color: rgb(p.background),
                text: rgb(p.text),
            };
            extended.background.weak = Pair {
                color: rgb(p.surface),
                text: rgb(p.text),
            };
            extended.background.strong = Pair {
                color: rgb(p.secondary),
                text: rgb(p.muted),
            };
            extended.primary.base = Pair {
                color: rgb(p.primary),
                text: rgb(p.primary_text),
            };
            extended.primary.strong = Pair {
                color: rgb(p.accent),
                text: rgb(p.primary_text),
            };
            extended.primary.weak = Pair {
                color: rgb(p.selection),
                text: rgb(p.accent),
            };
            extended.secondary.base = Pair {
                color: rgb(p.border),
                text: rgb(p.muted),
            };
            extended.danger.strong = Pair {
                color: rgb(p.flag),
                text: rgb(p.primary_text),
            };
            extended
        },
    )
}

pub(super) struct Editor {
    pub dark: bool,
    pub role: Role,
    pub value: String,
    pub draft: Option<Palettes>,
    changed: [u16; 2],
    pub error: Option<&'static str>,
}
impl Default for Editor {
    fn default() -> Self {
        Self {
            dark: false,
            role: Role::Primary,
            value: String::new(),
            draft: None,
            changed: [0; 2],
            error: None,
        }
    }
}
impl Editor {
    fn palette(&self, saved: Palettes) -> Palette {
        self.project(saved).get(self.dark)
    }
    fn selected_value(&self, saved: Palettes) -> String {
        self.palette(saved).get(self.role).to_string()
    }
    pub fn select(&mut self, dark: bool, role: Role, saved: Palettes) {
        self.dark = dark;
        self.role = role;
        self.value = self.selected_value(saved);
        self.error = None;
    }
    pub fn edit(&mut self, value: String, saved: Palettes) {
        self.error = if let Some(rgb) = Rgb::parse(&value) {
            self.changed[usize::from(self.dark)] |= 1 << self.role as u16;
            self.draft
                .get_or_insert(saved)
                .get_mut(self.dark)
                .set(self.role, rgb);
            None
        } else {
            Some("Use six hex digits, for example #7356BD.")
        };
        self.value = value;
    }
    pub fn reset(&mut self, saved: Palettes) {
        self.changed[usize::from(self.dark)] = u16::MAX;
        *self.draft.get_or_insert(saved).get_mut(self.dark) = Palette::defaults(self.dark);
        self.value = self.selected_value(saved);
        self.error = None;
    }
    fn project(&self, mut current: Palettes) -> Palettes {
        if let Some(draft) = self.draft {
            for dark in [false, true] {
                for &role in Role::ALL {
                    if self.changed[usize::from(dark)] & (1 << role as u16) != 0 {
                        current.get_mut(dark).set(role, draft.get(dark).get(role));
                    }
                }
            }
        }
        current
    }
    pub fn apply(&mut self, current: Palettes) -> Option<Palettes> {
        if self.error.is_some() || self.draft.is_none() {
            return None;
        }
        let current = self.project(current);
        self.draft = None;
        self.changed = [0; 2];
        self.value = current.get(self.dark).get(self.role).to_string();
        Some(current)
    }
    pub fn discard(&mut self, saved: Palettes) {
        self.draft = None;
        self.changed = [0; 2];
        self.value = self.selected_value(saved);
        self.error = None;
    }
}

impl App {
    pub(super) fn palette_settings(&self) -> Element<'_, Message> {
        let editor = &self.palette_editor;
        let p = editor.palette(self.preferences.palettes);
        let value = if editor.value.is_empty() || (editor.draft.is_none() && editor.error.is_none())
        {
            p.get(editor.role).to_string()
        } else {
            editor.value.clone()
        };
        let mut swatches = row![].spacing(6);
        for raw in [
            "#7356BD", "#2563EB", "#007F73", "#398366", "#C7954A", "#B91C1C", "#FFFFFF", "#F4F4F7",
            "#777580", "#292830", "#141416", "#000000",
        ] {
            let color = hex(Rgb::parse(raw).unwrap().value());
            swatches =
                swatches.push(
                    button(container(space()).width(26).height(24).style(move |_| {
                        container::Style {
                            background: Some(color.into()),
                            border: Border {
                                radius: 5.into(),
                                ..Default::default()
                            },
                            ..Default::default()
                        }
                    }))
                    .padding(6)
                    .style(outline)
                    .on_press(Message::PaletteValue(raw.into())),
                );
        }
        let foreground = hex(p.text.value());
        let muted_color = hex(p.muted.value());
        let sample = container(
            column![
                row![
                    text("Inbox").font(BOLD).size(18).color(foreground),
                    space().width(Length::Fill),
                    text("3 unread").size(12).color(muted_color)
                ]
                .align_y(Alignment::Center),
                container(
                    column![
                        text("Your next adventure")
                            .font(BOLD)
                            .size(14)
                            .color(foreground),
                        text("A little inspiration for the weekend.")
                            .size(12)
                            .color(muted_color)
                    ]
                    .spacing(6)
                )
                .padding(14)
                .width(Length::Fill)
                .style(move |_| container::Style {
                    background: Some(hex(p.surface.value()).into()),
                    border: Border {
                        radius: 8.into(),
                        width: 1.,
                        color: hex(p.border.value())
                    },
                    ..Default::default()
                }),
                row![
                    container(
                        text("New message")
                            .size(12)
                            .color(hex(p.primary_text.value()))
                    )
                    .padding([9, 14])
                    .style(move |_| container::Style {
                        background: Some(hex(p.primary.value()).into()),
                        border: Border {
                            radius: 7.into(),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
                    container(text("Selected").size(12).color(hex(p.accent.value())))
                        .padding([9, 14])
                        .style(move |_| container::Style {
                            background: Some(hex(p.selection.value()).into()),
                            border: Border {
                                radius: 7.into(),
                                ..Default::default()
                            },
                            ..Default::default()
                        }),
                    text("Flagged").size(12).color(hex(p.flag.value()))
                ]
                .spacing(12)
                .align_y(Alignment::Center)
            ]
            .spacing(14),
        )
        .padding(18)
        .width(Length::Fill)
        .style(move |_| container::Style {
            background: Some(hex(p.background.value()).into()),
            border: Border {
                radius: 9.into(),
                width: 1.,
                color: hex(p.border.value()),
            },
            ..Default::default()
        });
        let can_apply = editor.error.is_none()
            && editor
                .draft
                .is_some_and(|draft| draft != self.preferences.palettes);
        let warning: Element<'_, Message> = match editor.error.or(p.readability_warning()) {
            Some(message) => text(message)
                .size(12)
                .color(if self.dark() {
                    hex(0xFBBF24)
                } else {
                    hex(0xB45309)
                })
                .into(),
            None => muted("Preview your changes, then apply them to the app.")
                .size(12)
                .into(),
        };
        // Keep the editor legible even after a user deliberately saves low contrast.
        // The sample above is the only region styled using the unsaved palette.
        let card = self.settings_card(
            "Colors",
            "Customize light and dark palettes separately.",
            column![
                row![
                    button(text("Light palette").size(13))
                        .padding([10, 14])
                        .style(if editor.dark { outline } else { selected })
                        .on_press(Message::PaletteTheme(false)),
                    button(text("Dark palette").size(13))
                        .padding([10, 14])
                        .style(if editor.dark { selected } else { outline })
                        .on_press(Message::PaletteTheme(true)),
                ]
                .spacing(8),
                row![
                    pick_list(Role::ALL, Some(editor.role), Message::PaletteRole)
                        .style(select_input)
                        .padding(10)
                        .width(Length::Fill),
                    input("#7356BD", &value, Message::PaletteValue).width(130)
                ]
                .spacing(12)
                .align_y(Alignment::Center),
                swatches.wrap(),
                sample,
                warning,
                row![
                    action("Reset palette", Message::PaletteReset),
                    action("Undo changes", Message::PaletteDiscard),
                    space().width(if self.size.width < 1100. {
                        Length::Fixed(0.)
                    } else {
                        Length::Fill
                    }),
                    button(text("Apply colors").size(13))
                        .padding([10, 14])
                        .style(primary)
                        .on_press_maybe(can_apply.then_some(Message::PaletteApply))
                ]
                .spacing(8)
            ]
            .spacing(16)
            .into(),
        );
        static DEFAULTS: std::sync::OnceLock<[Theme; 2]> = std::sync::OnceLock::new();
        let defaults = DEFAULTS.get_or_init(|| {
            [
                make_theme(Palette::LIGHT, false),
                make_theme(Palette::DARK, true),
            ]
        });
        iced::widget::themer(Some(defaults[usize::from(self.dark())].clone()), card).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn palette_global_save_commits_staged_values_and_validates_before_confirmation() {
        let (mut app, _) = App::new();
        let _ = app.update(Message::PaletteValue("#007F73".into()));
        let _ = app.update(Message::SavePreferences);
        assert_eq!(
            app.preferences.palettes.light.primary.to_string(),
            "#007F73"
        );
        assert!(app.palette_editor.draft.is_none());
        let generation = app.preference_sync.generation();
        app.saved_toast = Some(Instant::now());
        let _ = app.update(Message::PaletteValue("#bad-input".into()));
        let _ = app.update(Message::SavePreferences);
        assert_eq!(app.preference_sync.generation(), generation);
        assert!(app.confirm_save.is_none());
        assert!(app.saved_toast.is_none());
        assert!(app.notice.as_ref().unwrap().0.contains("six hex digits"));
        assert_eq!(
            app.preferences.palettes.light.primary.to_string(),
            "#007F73"
        );
    }
    #[test]
    fn palette_all_color_roles_reach_the_component_theme() {
        let mut p = Palette::LIGHT;
        for (index, &role) in Role::ALL.iter().enumerate() {
            p.set(
                role,
                Rgb::parse(&format!("#{:06X}", 0x123456 + index * 0x010101)).unwrap(),
            );
        }
        let theme = make_theme(p, false);
        let c = colors(&theme);
        for (actual, expected) in [
            (c.bg, p.background),
            (c.surface, p.surface),
            (c.subtle, p.secondary),
            (c.text, p.text),
            (c.muted, p.muted),
            (c.border, p.border),
            (c.accent, p.accent),
            (c.tint, p.selection),
            (c.flag, p.flag),
            (theme.palette().success, p.success),
            (theme.palette().warning, p.warning),
            (theme.palette().danger, p.danger),
            (
                primary(&theme, iced::widget::button::Status::Active).text_color,
                p.primary_text,
            ),
        ] {
            assert_eq!(actual, hex(expected.value()));
        }
        assert_eq!(
            primary(&theme, iced::widget::button::Status::Active).background,
            Some(hex(p.primary.value()).into())
        );
    }
    #[test]
    fn palette_apply_preserves_unedited_colors_and_explicit_reversion() {
        let original = Palettes::default();
        let mut editor = Editor::default();
        editor.edit("#007F73".into(), original);
        let mut current = original;
        current.dark.background = Rgb::parse("#001122").unwrap();
        current.light.flag = Rgb::parse("#FF0000").unwrap();
        assert_eq!(editor.project(current).dark, current.dark);
        assert_eq!(editor.project(current).light.flag, current.light.flag);
        let applied = editor.apply(current).unwrap();
        assert_eq!(applied.light.primary.to_string(), "#007F73");
        assert_eq!(applied.dark, current.dark);
        assert_eq!(applied.light.flag, current.light.flag);
        editor.edit(original.light.primary.to_string(), applied);
        let mut remote = applied;
        remote.light.primary = Rgb::parse("#2563EB").unwrap();
        assert_eq!(
            editor.apply(remote).unwrap().light.primary,
            original.light.primary
        );
    }
    #[tokio::test]
    async fn palette_preferences_survive_store_reopen_and_legacy_defaults() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("palette.sqlite");
        let store = crate::store::Store::open(&path).unwrap();
        let mut preferences = Preferences::default();
        preferences.palettes.dark.accent = Rgb::parse("#33DDAA").unwrap();
        let palettes = preferences.palettes;
        store
            .update_preferences(move |p| p.palettes = palettes)
            .await
            .unwrap();
        drop(store);
        let reopened = crate::store::Store::open(&path).unwrap();
        let saved: Preferences = reopened.get("preferences").await.unwrap();
        assert_eq!(saved.palettes, preferences.palettes);
        let legacy: Preferences = serde_json::from_str("{\"appearance\":\"Dark\"}").unwrap();
        assert_eq!(legacy.palettes, Palettes::default());
    }
    #[test]
    fn palette_apply_is_optimistic_and_invalid_input_does_not_change_app() {
        let (mut app, _) = App::new();
        let original = app.preferences.palettes;
        let _ = app.update(Message::PaletteValue("#007F73".into()));
        assert_eq!(app.preferences.palettes, original);
        let _ = app.update(Message::PaletteApply);
        assert_eq!(
            app.preferences.palettes.light.primary.to_string(),
            "#007F73"
        );
        assert!(app.preference_sync.dirty());
        let _ = app.update(Message::PaletteValue("#not-a-color".into()));
        let _ = app.update(Message::PaletteApply);
        assert_eq!(
            app.preferences.palettes.light.primary.to_string(),
            "#007F73"
        );
        assert!(app.palette_editor.error.is_some());
    }
    #[test]
    fn palette_edit_validation_switch_and_reset_are_independent() {
        let saved = Palettes::default();
        let mut editor = Editor::default();
        editor.edit("#007F73".into(), saved);
        editor.select(true, Role::Background, saved);
        editor.edit("#101A24".into(), saved);
        editor.edit("#bad-input".into(), saved);
        assert!(editor.error.is_some());
        assert_eq!(editor.draft.unwrap().dark.background.to_string(), "#101A24");
        editor.reset(saved);
        assert!(editor.error.is_none());
        assert_eq!(editor.draft.unwrap().dark, Palette::DARK);
        assert_eq!(editor.draft.unwrap().light.primary.to_string(), "#007F73");
        editor.discard(saved);
        assert!(editor.draft.is_none());
    }
    #[test]
    fn palette_system_mode_uses_each_saved_palette_without_changing_the_choice() {
        let (mut app, _) = App::new();
        app.preferences.appearance = Appearance::System;
        app.preferences.palettes.light.background = Rgb::parse("#EFF6F2").unwrap();
        app.preferences.palettes.dark.background = Rgb::parse("#10221A").unwrap();
        app.system_dark = false;
        assert_eq!(app.theme().palette().background, hex(0xeff6f2));
        app.system_dark = true;
        assert_eq!(app.theme().palette().background, hex(0x10221a));
        assert_eq!(app.preferences.appearance, Appearance::System);
        app.preferences.appearance = Appearance::Light;
        assert_eq!(app.theme().palette().background, hex(0xeff6f2));
    }

    #[test]
    fn palette_theme_uses_explicit_mode_and_semantic_roles() {
        let mut palette = Palette::DARK;
        // A bright custom background must not silently switch all the other colors.
        palette.background = Rgb::parse("#EEEEEE").unwrap();
        palette.primary = Rgb::parse("#007F73").unwrap();
        let cache = ThemeCache::default();
        let theme = cache.get(palette, true);
        assert!(theme.extended_palette().is_dark);
        assert_eq!(theme.palette().background, hex(0xeeeeee));
        assert_eq!(theme.extended_palette().primary.base.color, hex(0x007f73));
        assert_eq!(
            theme.extended_palette().background.weak.color,
            hex(0x1b1b1f)
        );
        assert_eq!(theme, cache.get(palette, true));
        assert_ne!(theme, cache.get(Palette::LIGHT, false));
    }
}
