use serde::{Deserialize, Serialize};
use std::fmt::Write;
use windows::Win32::Foundation::COLORREF;

use crate::appearance::{EditorColors, luminance};
pub use crate::appearance::{Palette, readable};

pub const MIN_FONT_SIZE: i32 = 8;
pub const MAX_FONT_SIZE: i32 = 36;
pub const RICH_SIZE: i32 = 25;
pub const SOURCE_SIZE: i32 = 23;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct EditorFont {
    pub face: String,
    pub half_points: i32,
}

impl EditorFont {
    pub fn new(face: String, half_points: i32) -> Option<Self> {
        let font = Self { face, half_points };
        font.valid().then_some(font)
    }

    fn valid(&self) -> bool {
        (MIN_FONT_SIZE * 2..=MAX_FONT_SIZE * 2).contains(&self.half_points)
            && (1..32).contains(&self.face.encode_utf16().count())
            && !self.face.chars().any(|c| c == ';' || c.is_control())
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct EditorAppearance {
    pub rich: EditorFont,
    pub source: EditorFont,
    pub light: EditorColors,
    pub dark: EditorColors,
}

impl Default for EditorAppearance {
    fn default() -> Self {
        Self {
            rich: EditorFont::new("Segoe UI".into(), RICH_SIZE).unwrap(),
            source: EditorFont::new("Consolas".into(), SOURCE_SIZE).unwrap(),
            light: EditorColors::default(),
            dark: EditorColors::default(),
        }
    }
}

impl EditorAppearance {
    pub fn validate(&mut self) {
        if !self.rich.valid() {
            self.rich = Self::default().rich;
        }
        if !self.source.valid() {
            self.source = Self::default().source;
        }
        self.light.validate();
        self.dark.validate();
    }

    pub fn colors_mut(&mut self, dark: bool) -> &mut EditorColors {
        if dark {
            &mut self.dark
        } else {
            &mut self.light
        }
    }

    pub fn style(&self, dark: bool) -> EditorStyle<'_> {
        let colors = if dark { self.dark } else { self.light };
        let palette = Palette::new(dark).document(colors);
        EditorStyle {
            palette,
            rich: &self.rich,
            source: &self.source,
            dark: luminance(palette.canvas) < 0.179,
        }
    }
}

pub struct EditorStyle<'a> {
    pub palette: Palette,
    pub rich: &'a EditorFont,
    pub source: &'a EditorFont,
    pub dark: bool,
}

impl Palette {
    pub fn rgb(rgb: u32) -> COLORREF {
        COLORREF(((rgb & 255) << 16) | (rgb & 0xff00) | ((rgb >> 16) & 255))
    }
}

impl EditorStyle<'_> {
    pub fn rtf(&self) -> String {
        let mut out = String::from(r"{\rtf1\ansi\ansicpg1252\uc1\deff0{\fonttbl");
        for (index, font) in [self.rich, self.source].into_iter().enumerate() {
            let family = if index == 0 { "fnil" } else { "fmodern" };
            let _ = write!(out, "{{\\f{index}\\{family} ");
            let face = font.face.encode_utf16().collect::<Vec<_>>();
            super::rich::escaped(&mut out, &face);
            out.push_str(";}");
        }
        out.push_str(r"}{\colortbl;");
        let palette = self.palette;
        for color in [
            palette.text,
            palette.accent,
            palette.highlight,
            palette.marked,
            palette.line,
            palette.header,
            palette.muted,
            palette.canvas,
            palette.stripe,
        ] {
            let _ = write!(
                out,
                "\\red{}\\green{}\\blue{};",
                color >> 16,
                (color >> 8) & 255,
                color & 255
            );
        }
        out.push_str("}\n");
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editor_colors_are_independent_and_keep_custom_background_readable() {
        let settings = EditorAppearance {
            light: EditorColors {
                paper: Some(0x222b37),
                ink: Some(0x232b38),
                link: Some(0xffd97c),
            },
            ..EditorAppearance::default()
        };
        let style = settings.style(false);
        let p = style.palette;
        assert!(style.dark);
        assert_eq!(p.canvas, 0x222b37);
        assert_eq!(p.accent, 0xffd97c);
        assert_ne!(p.text, 0x232b38);
        assert_ne!(p.canvas, settings.style(true).palette.canvas);
        for ink in [p.text, p.accent, p.muted] {
            for background in [p.canvas, p.highlight, p.marked, p.header, p.stripe] {
                assert!(readable(ink, background), "{ink:06x} on {background:06x}");
            }
        }
    }
}
