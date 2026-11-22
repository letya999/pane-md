use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, OnceLock, mpsc},
    thread,
    time::{Duration, Instant},
};

use merman::{
    MermaidEngine, OperationControl, ParseOptions, RenderOutput, RenderRequest, Renderer,
    SourcePresentationConfig, SvgEnvironment, SvgRequest,
    svg::{RenderResourcePolicy, SvgPipeline},
};
use resvg::{tiny_skia, usvg};

#[derive(Debug, PartialEq)]
pub struct Image {
    pub png: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

pub struct Diagrams<P> {
    dark: bool,
    sources: Vec<Arc<str>>,
    pictures: HashMap<Arc<str>, Option<(P, usize)>>,
    job: Option<Job<P>>,
}

impl<P> Default for Diagrams<P> {
    fn default() -> Self {
        Self {
            dark: false,
            sources: Vec::new(),
            pictures: HashMap::new(),
            job: None,
        }
    }
}

type Pictures<P> = Vec<(Arc<str>, Option<(P, usize)>)>;

struct Job<P> {
    control: OperationControl,
    results: mpsc::Receiver<(Pictures<P>, bool)>,
}

impl<P> Drop for Job<P> {
    fn drop(&mut self) {
        self.control.cancel();
    }
}

const MAX_SOURCE_BYTES: usize = 64 * 1024;
const MAX_PREVIEW_BYTES: usize = 24 * 1024 * 1024;

pub fn select_sources<'a>(sources: impl Iterator<Item = &'a str>) -> Vec<&'a str> {
    let mut seen = HashSet::new();
    let mut remaining = 1024 * 1024;
    sources
        .filter(|&source| {
            if source.len() > MAX_SOURCE_BYTES || source.len() > remaining || !seen.insert(source) {
                return false;
            }
            remaining -= source.len();
            true
        })
        .take(128)
        .collect()
}

impl<P: Send + 'static> Diagrams<P> {
    pub fn prepare<'a>(
        &mut self,
        sources: impl Iterator<Item = &'a str>,
        dark: bool,
        present: fn(Image) -> (P, usize),
        notify: impl Fn() + Send + 'static,
    ) {
        let wanted = select_sources(sources);
        if self.dark == dark
            && self
                .sources
                .iter()
                .map(AsRef::as_ref)
                .eq(wanted.iter().copied())
        {
            return;
        }
        self.job = None;
        if self.dark == dark {
            let wanted_set: HashSet<_> = wanted.iter().copied().collect();
            self.pictures
                .retain(|source, _| wanted_set.contains(source.as_ref()));
        } else {
            self.dark = dark;
            self.pictures.clear();
        }
        self.sources = wanted.into_iter().map(Arc::<str>::from).collect();
        let missing = self
            .sources
            .iter()
            .filter(|source| !self.pictures.contains_key(*source))
            .cloned()
            .collect::<Vec<_>>();
        if missing.is_empty() {
            return;
        }
        let cached = self
            .pictures
            .values()
            .filter_map(Option::as_ref)
            .map(|(_, bytes)| *bytes)
            .sum::<usize>();
        let (tx, results) = mpsc::channel();
        let control = OperationControl::new();
        let worker_control = control.clone();
        if thread::Builder::new()
            .name("mermaid-preview".into())
            .spawn(move || {
                let remaining = MAX_PREVIEW_BYTES.saturating_sub(cached);
                render_batch(
                    missing,
                    dark,
                    remaining,
                    worker_control,
                    present,
                    |batch, finished| {
                        if tx.send((batch, finished)).is_ok() {
                            notify();
                        }
                    },
                );
            })
            .is_ok()
        {
            self.job = Some(Job { control, results });
        }
    }

    pub fn receive(&mut self) -> bool {
        let Some(job) = &self.job else { return false };
        let mut changed = false;
        loop {
            match job.results.try_recv() {
                Ok((batch, finished)) => {
                    changed = true;
                    self.pictures.extend(batch);
                    if finished {
                        self.job = None;
                        break;
                    }
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.job = None;
                    changed = true;
                    break;
                }
            }
        }
        changed
    }

    pub fn picture(&self, source: &str) -> (Option<&P>, bool) {
        match self.pictures.get(source) {
            Some(picture) => (picture.as_ref().map(|(value, _)| value), false),
            None => (
                None,
                self.job.is_some() && self.sources.iter().any(|value| value.as_ref() == source),
            ),
        }
    }
}

fn render_batch<P>(
    sources: Vec<Arc<str>>,
    dark: bool,
    mut remaining: usize,
    control: OperationControl,
    present: fn(Image) -> (P, usize),
    mut publish: impl FnMut(Pictures<P>, bool),
) {
    let total = sources.len();
    let mut batch = Vec::new();
    let mut last = None;
    for (index, source) in sources.into_iter().enumerate() {
        if control.is_cancelled() {
            return;
        }
        let picture = (remaining != 0)
            .then(|| render(&source, dark, &control))
            .flatten()
            .map(present)
            .filter(|(_, bytes)| *bytes <= remaining);
        remaining -= picture.as_ref().map_or(0, |(_, bytes)| *bytes);
        batch.push((source, picture));
        let finished = index + 1 == total;
        if finished || last.is_none_or(|at: Instant| at.elapsed() >= Duration::from_millis(80)) {
            publish(std::mem::take(&mut batch), finished);
            last = Some(Instant::now());
        }
    }
}

#[cfg(windows)]
const SYSTEM_FONT: &str = "Segoe UI";
#[cfg(target_os = "linux")]
const SYSTEM_FONT: &str = "DejaVu Sans";
#[cfg(target_os = "macos")]
const SYSTEM_FONT: &str = "Helvetica Neue";

pub(super) fn svg_options() -> &'static usvg::Options<'static> {
    static OPTIONS: OnceLock<usvg::Options<'static>> = OnceLock::new();
    OPTIONS.get_or_init(|| {
        let mut options = usvg::Options {
            font_family: SYSTEM_FONT.into(),
            image_href_resolver: usvg::ImageHrefResolver {
                resolve_data: Box::new(|_, _, _| None),
                resolve_string: Box::new(|_, _| None),
            },
            ..Default::default()
        };
        let fonts = options.fontdb_mut();
        fonts.load_system_fonts();
        fonts.set_sans_serif_family(SYSTEM_FONT);
        options
    })
}

fn renderer(dark: bool) -> &'static Renderer {
    static DARK: OnceLock<Renderer> = OnceLock::new();
    static LIGHT: OnceLock<Renderer> = OnceLock::new();
    let renderer = if dark { &DARK } else { &LIGHT };
    renderer.get_or_init(|| {
        let site = SourcePresentationConfig::from_value(serde_json::json!({
            "theme": if dark { "dark" } else { "default" },
            "fontFamily": format!("{SYSTEM_FONT}, sans-serif"),
            "securityLevel": "strict",
            "maxTextSize": MAX_SOURCE_BYTES,
        }));
        Renderer::new()
            .with_presentation_engine(MermaidEngine::new().with_site_config(site))
            .with_parse_options(ParseOptions::strict())
    })
}

fn svg(source: &str, dark: bool, control: &OperationControl) -> Option<String> {
    let input = pane_md::normalize_newlines(source, "\n");
    let request = SvgRequest {
        environment: SvgEnvironment::deterministic()
            .with_resource_policy(RenderResourcePolicy::interactive())
            .with_compiled_math_renderer(),
        pipeline: Some(SvgPipeline::resvg_safe()),
        ..Default::default()
    };
    let engine = renderer(dark);
    let operation = control.child().with_deadline(Duration::from_millis(750));
    let output = engine
        .render(RenderRequest::svg(&input, operation, request))
        .ok()?;
    let RenderOutput::Svg(Some(output)) = output else {
        return None;
    };
    Some(output.into_parts().0)
}

fn render(source: &str, dark: bool, control: &OperationControl) -> Option<Image> {
    let svg = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| svg(source, dark, control)))
        .ok()??;
    if control.is_cancelled() {
        return None;
    }
    let tree = usvg::Tree::from_str(&svg, svg_options()).ok()?;
    let size = tree.size();
    let scale = (960.0 / size.width()).min(1200.0 / size.height()).min(1.0);
    let width = (size.width() * scale).ceil().max(1.0) as u32;
    let height = (size.height() * scale).ceil().max(1.0) as u32;
    let mut image = tiny_skia::Pixmap::new(width * 2, height * 2)?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale * 2.0, scale * 2.0),
        &mut image.as_mut(),
    );
    if image
        .data()
        .as_chunks::<4>()
        .0
        .iter()
        .all(|pixel| pixel[3] == 0)
    {
        return None;
    }
    Some(Image {
        png: image.encode_png().ok()?,
        width,
        height,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mermaid_families_render_as_png_images() {
        let sources = [
            "flowchart LR\nA[Open] --> B[Read]",
            "sequenceDiagram\nWriter->>Reader: Send note",
            "classDiagram\nclass Note {\n  +title String\n}",
            "stateDiagram-v2\n[*] --> Draft\nDraft --> Saved",
            "erDiagram\nAUTHOR ||--o{ NOTE : writes",
            "gantt\ntitle Release\ndateFormat YYYY-MM-DD\ntodayMarker off\nBuild :b1, 2026-11-20, 2d",
            "journey\ntitle First visit\nsection Start\nOpen note: 5: Reader",
            "timeline\ntitle Releases\n2025 : Preview\n2026 : Stable",
            "pie showData\n\"Notes\": 8\n\"Readmes\": 3",
            "gitGraph\ncommit id: \"Start\"\nbranch docs\ncommit id: \"Write\"",
            "sankey\nDraft, Review, 8\nReview, Published, 6",
            "quadrantChart\nx-axis Slow --> Fast\ny-axis Low --> High\nEditor: [0.7, 0.6]",
            "xychart-beta\nx-axis [Mon, Tue, Wed]\nbar [1, 4, 2]",
            "packet\n0-3: \"Version\"\n4-15: \"Length\"",
            "mindmap\n  root((Notes))\n    Drafts\n    Published",
            "requirementDiagram\nrequirement ready {\n  id: 1\n  text: File opens\n  risk: low\n}",
            "C4Context\nPerson(reader, \"Reader\")\nSystem(editor, \"Editor\")\nRel(reader, editor, \"Reads\")",
            "block-beta\ncolumns 2\nA[\"Editor\"] B[\"File\"]\nA --> B",
            "zenuml\nEditor->Disk: Save\nDisk-->Editor: Done",
            "info\nshowInfo",
            "agentflow-beta TB\nflow review[\"Note review\"]\n input[\"Draft\"]@{ shape: input }\n task[\"Check\"]@{ shape: task }\n input --> task\nend",
            "usecase-beta\nactor Reader\nReader --> Open",
            "architecture-beta\nservice web(server)[Web]\nservice db(database)[Data]\nweb:R --> L:db",
            "venn-beta\nset A[\"Review\"]:8\nset B[\"Editing\"]:6\nunion A,B[\"Shared\"]:2",
            "radar-beta\naxis A, B, C\ncurve Run {3,2,4}",
            "railroad-beta\nentry = sequence(terminal(\"on\"), terminal(\"off\")) ;",
            "treeView-beta\n\"docs\"\n    \"guide\"\n    \"reference\"",
            "cynefin-beta\nclear\n  \"Routine\"\ncomplex\n  \"Research\"",
            "swimlane-beta LR\nsubgraph Review\nA[Draft] --> B[Decision]\nend",
            "eventmodeling\ntf 01 ui Notes.Home\ntf 02 cmd Notes.Open ->> 01",
            "ishikawa-beta\nSlow response\n  Process\n    Multiple handoffs",
            "wardley-beta\ntitle Note service\nanchor Reader [0.95, 0.2]\ncomponent Editor [0.7, 0.4]\nReader -> Editor",
            "treemap-beta\n\"Docs\"\n  \"Tutorials\": 7\n  \"Reference\": 9",
            "kanban\nreview[Review]\n  request[Check examples]",
        ];
        assert_eq!(sources.len(), merman::diagram_family_selectors().len());
        for source in sources {
            let picture = render(source, false, &OperationControl::new())
                .unwrap_or_else(|| panic!("diagram did not render: {source}"));
            assert!(picture.png.starts_with(b"\x89PNG\r\n\x1a\n"), "{source}");
        }
    }

    #[test]
    fn elk_and_math_are_available_in_the_preview_pipeline() {
        let source = "---\nconfig:\n  layout: elk\n---\nflowchart LR\nA[\"$$x^2$$\"] --> B[Result]";
        let control = OperationControl::new();
        let svg = svg(source, false, &control).expect("ELK diagram with a formula");
        assert!(!svg.contains("$$x^2$$"));
        assert!(!svg.contains("data-merman-math-native=\"unavailable\""));
        assert!(render(source, false, &control).is_some());
    }

    #[test]
    fn previews_follow_editor_theme_and_failed_input_remains_source() {
        let control = OperationControl::new();
        let source = "flowchart LR\r\n A[Текст] --> B[Перевод]";
        let light = render(source, false, &control).expect("light diagram");
        let dark = render(source, true, &control).expect("dark diagram");
        assert_ne!(light, dark);
        assert!(render("not-a-diagram\r\n input", false, &control).is_none());
        control.cancel();
        assert!(render(source, false, &control).is_none());
    }
}
