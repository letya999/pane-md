use std::{
    borrow::Cow,
    collections::HashMap,
    sync::{Arc, RwLock},
};

use pane_md::markdown::Markdown;
use serde::Serialize;
use wry::http::{Method, Request, Response};

use crate::diagram::{Diagrams, Image, select_sources};

type Pictures = HashMap<usize, Arc<Image>>;

#[derive(Clone, Default)]
pub struct Assets(Arc<RwLock<(u64, Pictures)>>);

impl Assets {
    pub fn response(&self, request: &Request<Vec<u8>>) -> Option<Response<Cow<'static, [u8]>>> {
        let uri = request.uri();
        if request.method() != Method::GET
            || uri.scheme_str() != Some("panemd")
            || uri.authority()?.as_str() != "app"
            || uri.query().is_some()
        {
            return None;
        }
        let (generation, region) = uri.path().strip_prefix("/diagrams/")?.split_once('/')?;
        let generation: u64 = generation.parse().ok()?;
        let region = region.strip_suffix(".png")?.parse().ok()?;
        let assets = self.0.read().ok()?;
        if assets.0 != generation {
            return None;
        }
        let image = assets.1.get(&region)?;
        Response::builder()
            .header("Content-Type", "image/png")
            .header("Cache-Control", "no-store")
            .body(Cow::Owned(image.png.clone()))
            .ok()
    }
}

#[derive(Default)]
pub struct View {
    engine: Diagrams<Arc<Image>>,
    slots: Vec<(usize, Arc<str>)>,
    revision: u64,
    generation: u64,
    dark: bool,
    pub assets: Assets,
}

#[derive(Serialize)]
pub struct Snapshot {
    revision: u64,
    entries: Vec<Entry>,
}

#[derive(Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
enum Entry {
    Ready {
        region: usize,
        url: String,
        width: u32,
        height: u32,
    },
    Pending {
        region: usize,
    },
    Fallback {
        region: usize,
    },
}

impl View {
    pub fn prepare(
        &mut self,
        doc: Option<&Markdown>,
        dark: bool,
        notify: impl Fn() + Send + 'static,
    ) {
        self.revision += 1;
        let regions = || {
            doc.into_iter()
                .flat_map(|doc| doc.regions.iter().enumerate())
                .filter(|(_, region)| region.mermaid)
                .map(|(index, region)| (index, region.text.as_str()))
        };
        let sources: HashMap<_, _> = select_sources(regions().map(|(_, source)| source))
            .into_iter()
            .map(|source| (source, Arc::<str>::from(source)))
            .collect();
        let slots: Vec<_> = regions()
            .filter_map(|(region, source)| Some((region, sources.get(source)?.clone())))
            .collect();
        if self.slots != slots {
            self.slots = slots;
            self.generation += 1;
        }
        self.set_theme(dark, notify);
    }

    pub fn set_theme(&mut self, dark: bool, notify: impl Fn() + Send + 'static) {
        if self.dark != dark {
            self.dark = dark;
            self.generation += 1;
        }
        self.engine.prepare(
            self.slots.iter().map(|(_, source)| source.as_ref()),
            dark,
            |image| {
                let size = image.png.len();
                (Arc::new(image), size)
            },
            notify,
        );
    }

    pub fn receive(&mut self) -> bool {
        self.engine.receive()
    }

    pub fn snapshot(&self) -> Snapshot {
        let mut pictures = HashMap::new();
        let entries = self
            .slots
            .iter()
            .map(|(region, source)| match self.engine.picture(source) {
                (Some(image), _) => {
                    pictures.insert(*region, image.clone());
                    Entry::Ready {
                        region: *region,
                        url: format!("/diagrams/{}/{region}.png", self.generation),
                        width: image.width,
                        height: image.height,
                    }
                }
                (None, true) => Entry::Pending { region: *region },
                (None, false) => Entry::Fallback { region: *region },
            })
            .collect();
        *self.assets.0.write().expect("diagram asset lock") = (self.generation, pictures);
        Snapshot {
            revision: self.revision,
            entries,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assets_serve_only_the_current_generation_on_the_private_origin() {
        let assets = Assets::default();
        let image = Arc::new(Image {
            png: vec![1, 2, 3],
            width: 1,
            height: 1,
        });
        *assets.0.write().unwrap() = (3, HashMap::from([(7, image)]));
        let request = |uri: &str| Request::get(uri).body(Vec::new()).unwrap();
        let valid = "panemd://app/diagrams/3/7.png";
        let response = assets.response(&request(valid)).unwrap();
        assert_eq!(response.headers()["Content-Type"], "image/png");
        assert_eq!(response.body().as_ref(), [1, 2, 3]);
        for uri in [
            "panemd://app/diagrams/2/7.png",
            "panemd://app/diagrams/3/9.png",
            "https://app/diagrams/3/7.png",
            "panemd://outside/diagrams/3/7.png",
            "panemd://app/diagrams/3/7.png?other",
            "panemd://app/diagrams/3/%2e%2e/7.png",
        ] {
            assert!(assets.response(&request(uri)).is_none(), "{uri}");
        }
        assert!(
            assets
                .response(&Request::post(valid).body(Vec::new()).unwrap())
                .is_none()
        );
    }

    #[test]
    fn replaces_previews_by_theme_and_ignores_work_for_closed_documents() {
        use std::{sync::mpsc, time::Duration};
        let mut view = View::default();
        let doc =
            Markdown::parse("```mermaid\nflowchart LR\nA --> B\n```\n\n```mermaid\nbad\n```\n");
        let (tx, rx) = mpsc::channel();
        let notification = {
            let tx = tx.clone();
            move || {
                let _ = tx.send(());
            }
        };
        view.prepare(Some(&doc), false, notification);
        while view
            .snapshot()
            .entries
            .iter()
            .any(|entry| matches!(entry, Entry::Pending { .. }))
        {
            rx.recv_timeout(Duration::from_secs(5)).unwrap();
            view.receive();
        }
        let light = view.snapshot();
        let [Entry::Ready { url, .. }, Entry::Fallback { .. }] = light.entries.as_slice() else {
            panic!("expected a diagram and its invalid neighbor as source");
        };
        let request = Request::get(format!("panemd://app{url}"))
            .body(Vec::new())
            .unwrap();
        assert!(view.assets.response(&request).is_some());

        view.set_theme(true, move || {
            let _ = tx.send(());
        });
        let dark = view.snapshot();
        assert_eq!(dark.revision, light.revision);
        assert!(view.assets.response(&request).is_none());

        view.prepare(None, true, || {});
        assert!(!view.receive());
        let closed = view.snapshot();
        assert!(closed.entries.is_empty());
        assert_ne!(closed.revision, dark.revision);
    }
}
