use std::{collections::BTreeSet, fmt::Write};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    appearance::{EditorColors, Palette, luminance},
    diagram::svg_options,
    i18n::{Label as L, tr},
};

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Font {
    pub family: String,
    pub half_points: u8,
}

impl Font {
    fn system(half_points: u8) -> Self {
        Self {
            family: String::new(),
            half_points,
        }
    }

    fn valid(&self) -> bool {
        (16..=72).contains(&self.half_points)
            && self.family.len() <= 256
            && !self.family.chars().any(char::is_control)
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct Appearance {
    rich: Font,
    source: Font,
    light: EditorColors,
    dark: EditorColors,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            rich: Font::system(24),
            source: Font::system(23),
            light: EditorColors::default(),
            dark: EditorColors::default(),
        }
    }
}

impl Appearance {
    pub fn sanitize(&mut self) {
        if !self.rich.valid() {
            self.rich = Self::default().rich;
        }
        if !self.source.valid() {
            self.source = Self::default().source;
        }
        self.light.validate();
        self.dark.validate();
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        if !self.rich.valid() || !self.source.valid() {
            return Err(tr(L::InvalidFont));
        }
        for colors in [self.light, self.dark] {
            if [colors.paper, colors.ink, colors.link]
                .into_iter()
                .flatten()
                .any(|color| color > 0xffffff)
            {
                return Err(tr(L::InvalidColor));
            }
        }
        Ok(())
    }

    fn palette(&self, dark: bool) -> Palette {
        Palette::new(dark).document(if dark { self.dark } else { self.light })
    }

    pub fn diagram_dark(&self, dark: bool) -> bool {
        let overrides = if dark { self.dark } else { self.light };
        luminance(overrides.paper.unwrap_or(Palette::new(dark).canvas)) < 0.179
    }

    pub fn snapshot(&self) -> Value {
        json!({
            "settings": self,
            "light": self.palette(false),
            "dark": self.palette(true),
            "defaults": {
                "settings": Self::default(),
                "light": Palette::new(false),
                "dark": Palette::new(true),
            },
        })
    }
}

pub fn installed_fonts() -> Vec<String> {
    svg_options()
        .fontdb
        .faces()
        .filter_map(|face| face.families.first().map(|(family, _)| family))
        .filter(|family| family.len() <= 256 && !family.chars().any(char::is_control))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .cloned()
        .collect()
}

pub fn theme_css() -> String {
    let values = |dark| {
        let p = Palette::new(dark);
        let mut css = format!("color-scheme:{};", if dark { "dark" } else { "light" });
        for (name, value) in [
            ("paper", p.canvas),
            ("frame", p.frame),
            ("text", p.text),
            ("muted", p.muted),
            ("line", p.line),
            ("surface", p.header),
            ("accent", p.accent),
            ("selected", p.highlight),
            ("caption", p.caption),
            ("caption-hover", p.caption_hover),
            ("on-caption", p.on_caption),
            ("action", p.action),
            ("on-action", p.on_action),
        ] {
            let _ = write!(css, "--{name}:#{value:06x};");
        }
        css
    };
    let light = values(false);
    let dark = values(true);
    format!(
        ":root{{{light}}}[data-theme=dark]{{{dark}}}@media(prefers-color-scheme:dark){{[data-theme=system]{{{dark}}}}}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::appearance::readable;

    #[test]
    fn custom_theme_keeps_all_editor_colors_readable() {
        let appearance = Appearance {
            light: EditorColors {
                paper: Some(0x202020),
                ink: Some(0x212121),
                link: Some(0x22eeee),
            },
            ..Appearance::default()
        };
        let palette = appearance.palette(false);
        assert_eq!(palette.canvas, 0x202020);
        assert_eq!(palette.accent, 0x22eeee);
        assert!(appearance.diagram_dark(false));
        for foreground in [palette.text, palette.accent, palette.muted] {
            for background in [
                palette.canvas,
                palette.header,
                palette.stripe,
                palette.highlight,
                palette.marked,
            ] {
                assert!(readable(foreground, background));
            }
        }
        assert_eq!(appearance.palette(true).canvas, Palette::new(true).canvas);
    }

    #[test]
    fn bad_values_are_rejected_and_stored_defaults_are_recoverable() {
        let mut appearance: Appearance = serde_json::from_str(
            r#"{"rich":{"family":"Invalid font","half_points":255},"light":{"paper":16777216}}"#,
        )
        .unwrap();
        assert!(appearance.validate().is_err());
        appearance.sanitize();
        assert_eq!(appearance, Appearance::default());
    }
}
