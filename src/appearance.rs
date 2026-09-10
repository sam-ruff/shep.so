//! Portable opaque RGB colors. Parsing and presentation are separate from I/O.
pub mod edits;
use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb(u32);
impl Rgb {
    pub const fn value(self) -> u32 {
        self.0
    }
    pub fn parse(value: &str) -> Option<Self> {
        let value = value.trim().strip_prefix('#').unwrap_or(value.trim());
        (value.len() == 6 && value.bytes().all(|b| b.is_ascii_hexdigit()))
            .then(|| Self(u32::from_str_radix(value, 16).unwrap()))
    }
    pub fn contrast(self, other: Self) -> f32 {
        let luminance = |rgb: Self| {
            let linear = |shift: u32| {
                let n = ((rgb.0 >> shift) & 255u32) as f32 / 255.;
                if n <= 0.04045 {
                    n / 12.92
                } else {
                    ((n + 0.055) / 1.055).powf(2.4)
                }
            };
            linear(16) * 0.2126 + linear(8) * 0.7152 + linear(0) * 0.0722
        };
        let (a, b) = (luminance(self), luminance(other));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }
}
impl std::fmt::Display for Rgb {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "#{:06X}", self.0)
    }
}
impl Serialize for Rgb {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}
impl<'de> Deserialize<'de> for Rgb {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value)
            .ok_or_else(|| D::Error::custom("Use a six-digit RGB color, such as #7356BD"))
    }
}

macro_rules! roles {
    ($( $role:ident, $field:ident, $label:literal, $light:literal, $dark:literal; )+) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum Role { $( $role ),+ }
        impl Role { pub const ALL: &'static [Self] = &[ $( Self::$role ),+ ]; }
        impl std::fmt::Display for Role {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(match self { $( Self::$role => $label ),+ })
            }
        }
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
        pub struct Palette { $( pub $field: Rgb ),+ }
        impl Palette {
            pub const LIGHT: Self = Self { $( $field: Rgb($light) ),+ };
            pub const DARK: Self = Self { $( $field: Rgb($dark) ),+ };
            pub fn get(self, role: Role) -> Rgb { match role { $( Role::$role => self.$field ),+ } }
            pub fn set(&mut self, role: Role, value: Rgb) { match role { $( Role::$role => self.$field = value ),+ } }
            pub const fn defaults(dark: bool) -> Self { if dark { Self::DARK } else { Self::LIGHT } }
        }
    };
}
roles! {
    Primary, primary, "Primary · buttons", 0x7356bd, 0x7356bd;
    PrimaryText, primary_text, "Primary button text", 0xffffff, 0xffffff;
    Secondary, secondary, "Secondary · subtle surfaces", 0xf4f4f7, 0x232329;
    Background, background, "Background", 0xf7f7f9, 0x141416;
    Surface, surface, "Surface · cards and reader", 0xffffff, 0x1b1b1f;
    Text, text, "Text", 0x292830, 0xf4f4f5;
    Muted, muted, "Muted text", 0x777580, 0xa5a5b0;
    Border, border, "Borders", 0xe8e7ed, 0x323239;
    Accent, accent, "Accent · links and focus", 0x7356bd, 0xb5a0ff;
    Selection, selection, "Selection background", 0xf0eafa, 0x30283f;
    Flag, flag, "Flagged mail", 0xc62828, 0xf87171;
    Success, success, "Success", 0x398366, 0x398366;
    Warning, warning, "Warning", 0xc7954a, 0xc7954a;
    Danger, danger, "Danger · destructive actions", 0xb91c1c, 0xb91c1c;
}
impl Palette {
    pub fn readability_warning(self) -> Option<&'static str> {
        if self
            .text
            .contrast(self.background)
            .min(self.text.contrast(self.surface))
            < 3.
        {
            Some("Text may be hard to read against the background or surface.")
        } else if self.primary_text.contrast(self.primary) < 3. {
            Some("Button text may be hard to read against the primary color.")
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Palettes {
    pub light: Palette,
    pub dark: Palette,
}
impl Default for Palettes {
    fn default() -> Self {
        Self {
            light: Palette::LIGHT,
            dark: Palette::DARK,
        }
    }
}
impl Palettes {
    pub fn get(self, dark: bool) -> Palette {
        if dark { self.dark } else { self.light }
    }
    pub fn get_mut(&mut self, dark: bool) -> &mut Palette {
        if dark {
            &mut self.dark
        } else {
            &mut self.light
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn palette_round_trip_defaults_and_invalid_colors() {
        let mut palettes = Palettes::default();
        palettes
            .dark
            .set(Role::Primary, Rgb::parse(" #008877 ").unwrap());
        let json = serde_json::to_string(&palettes).unwrap();
        assert!(json.contains("#008877"));
        assert_eq!(serde_json::from_str::<Palettes>(&json).unwrap(), palettes);
        assert_eq!(palettes.light, Palette::LIGHT);
        assert_eq!(
            serde_json::from_str::<Palettes>("{}").unwrap(),
            Palettes::default()
        );
        for bad in ["#fff", "#FFFFFF00", "-12345", "#ééé", "#zzzzzz", "1234567"] {
            assert!(Rgb::parse(bad).is_none());
            assert!(serde_json::from_value::<Rgb>(serde_json::json!(bad)).is_err());
        }
    }
    #[test]
    fn palette_readability_reports_actual_text_pairs() {
        assert!(Palette::LIGHT.readability_warning().is_none());
        assert!(Palette::DARK.readability_warning().is_none());
        let mut palette = Palette::LIGHT;
        palette.text = palette.surface;
        assert!(palette.readability_warning().unwrap().starts_with("Text"));
        palette = Palette::DARK;
        palette.primary_text = palette.primary;
        assert!(palette.readability_warning().unwrap().starts_with("Button"));
    }
}
