use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct EditorColors {
    pub paper: Option<u32>,
    pub ink: Option<u32>,
    pub link: Option<u32>,
}

impl EditorColors {
    pub fn validate(&mut self) {
        for color in [&mut self.paper, &mut self.ink, &mut self.link] {
            *color = color.filter(|&value| value <= 0xffffff);
        }
    }
}

#[derive(Clone, Copy)]
#[cfg_attr(not(windows), derive(Serialize))]
pub struct Palette {
    pub canvas: u32,
    pub frame: u32,
    pub text: u32,
    pub muted: u32,
    pub line: u32,
    pub accent: u32,
    pub highlight: u32,
    pub marked: u32,
    pub header: u32,
    pub stripe: u32,
    pub caption: u32,
    pub caption_hover: u32,
    pub on_caption: u32,
    pub action: u32,
    pub on_action: u32,
}

impl Palette {
    pub fn new(dark: bool) -> Self {
        if dark {
            Self {
                canvas: 0x1e2125,
                frame: 0x282c31,
                text: 0xe5e9ed,
                muted: 0xb5bec7,
                line: 0x535c65,
                accent: 0x87c7f3,
                highlight: 0x244973,
                marked: 0x2d4057,
                header: 0x2d353d,
                stripe: 0x24282d,
                caption: 0x242627,
                caption_hover: 0x3c3f42,
                on_caption: 0xf7f7f7,
                action: 0x8bc5ff,
                on_action: 0x18202a,
            }
        } else {
            Self {
                canvas: 0xfbfaf8,
                frame: 0xf1f0ee,
                text: 0x202326,
                muted: 0x555c63,
                line: 0xd5d4d1,
                accent: 0x205cb0,
                highlight: 0xcfe3fc,
                marked: 0xdfecfb,
                header: 0xe9e8e6,
                stripe: 0xf4f3f0,
                caption: 0x242627,
                caption_hover: 0x3c3f42,
                on_caption: 0xf7f7f7,
                action: 0x125ebf,
                on_action: 0xffffff,
            }
        }
    }

    pub fn document(self, colors: EditorColors) -> Self {
        if colors == EditorColors::default() {
            return self;
        }
        let paper = colors.paper.unwrap_or(self.canvas);
        let dark = luminance(paper) < 0.179;
        let mut palette = Self::new(dark);
        let neutral = if dark { 0xffffff } else { 0 };
        let legible = |preferred, fallback| {
            if readable(preferred, paper) {
                preferred
            } else {
                fallback
            }
        };
        palette.canvas = paper;
        palette.text = legible(
            colors.ink.unwrap_or(palette.text),
            legible(palette.text, neutral),
        );
        palette.accent = legible(
            colors.link.unwrap_or(palette.accent),
            legible(palette.accent, palette.text),
        );
        palette.muted = legible(palette.muted, palette.text);
        let shade = |stock, color, amount| {
            if paper == self.canvas
                && [palette.text, palette.accent, palette.muted]
                    .into_iter()
                    .all(|ink| readable(ink, stock))
            {
                return stock;
            }
            (1..=amount)
                .rev()
                .map(|percent| mix(paper, color, percent))
                .find(|&background| {
                    [palette.text, palette.accent, palette.muted]
                        .into_iter()
                        .all(|ink| readable(ink, background))
                })
                .unwrap_or(paper)
        };
        palette.highlight = shade(palette.highlight, palette.accent, 26);
        palette.marked = shade(palette.marked, palette.accent, 14);
        palette.header = shade(palette.header, palette.text, 9);
        palette.stripe = shade(palette.stripe, palette.text, 4);
        if paper != self.canvas {
            palette.line = mix(paper, neutral, 35);
        }
        palette
    }
}

pub fn luminance(rgb: u32) -> f64 {
    [(16, 0.2126), (8, 0.7152), (0, 0.0722)]
        .into_iter()
        .map(|(shift, weight)| {
            let channel = f64::from((rgb >> shift) & 255) / 255.0;
            weight
                * if channel <= 0.04045 {
                    channel / 12.92
                } else {
                    ((channel + 0.055) / 1.055).powf(2.4)
                }
        })
        .sum()
}

pub fn readable(ink: u32, paper: u32) -> bool {
    let ink = luminance(ink) + 0.05;
    let paper = luminance(paper) + 0.05;
    ink.max(paper) >= 4.5 * ink.min(paper)
}

fn mix(paper: u32, ink: u32, percent: u32) -> u32 {
    [16, 8, 0].into_iter().fold(0, |rgb, shift| {
        let base = (paper >> shift) & 255;
        let tint = (ink >> shift) & 255;
        rgb | (((base * (100 - percent) + tint * percent) / 100) << shift)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_palette_keeps_chrome_and_highlights_readable() {
        for dark in [false, true] {
            let p = Palette::new(dark);
            for (foreground, background) in [
                (p.text, p.canvas),
                (p.text, p.frame),
                (p.muted, p.frame),
                (p.muted, p.canvas),
                (p.accent, p.highlight),
                (p.text, p.highlight),
                (p.on_caption, p.caption),
                (p.on_action, p.action),
            ] {
                assert!(
                    readable(foreground, background),
                    "{foreground:06x} on {background:06x}"
                );
            }
        }
    }
}
