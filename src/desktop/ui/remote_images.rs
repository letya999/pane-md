use std::{
    collections::HashMap,
    sync::mpsc::{self, Receiver, Sender},
    time::Duration,
};

use windows::Win32::{
    Foundation::{HWND, LPARAM, WPARAM},
    UI::WindowsAndMessaging::PostMessageW,
};

use crate::images::{ImageFormat, MAX_BYTES, rasterize_svg};

use super::WM_DIAGRAM_READY;

const MAX_PENDING: usize = 4;
const CACHE_BYTES: usize = 24 * 1024 * 1024;

enum Entry {
    Pending,
    Ready(Vec<u8>),
    Failed,
}

pub(in crate::desktop) struct RemoteImages {
    entries: HashMap<String, Entry>,
    sender: Sender<(String, Option<Vec<u8>>)>,
    receiver: Receiver<(String, Option<Vec<u8>>)>,
}

impl Default for RemoteImages {
    fn default() -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            entries: HashMap::new(),
            sender,
            receiver,
        }
    }
}

impl RemoteImages {
    pub(in crate::desktop) fn picture(&self, url: &str) -> Option<&[u8]> {
        match self.entries.get(url) {
            Some(Entry::Ready(data)) => Some(data),
            _ => None,
        }
    }

    pub(super) fn request(&mut self, raw: &str, url: &str, hwnd: HWND) -> bool {
        let scheme = url.split_once(':').map(|(scheme, _)| scheme).unwrap_or("");
        if !scheme.eq_ignore_ascii_case("https") && !scheme.eq_ignore_ascii_case("http") {
            return false;
        }
        if matches!(
            self.entries.get(raw),
            Some(Entry::Pending | Entry::Ready(_))
        ) || self
            .entries
            .values()
            .filter(|entry| matches!(entry, Entry::Pending))
            .count()
            >= MAX_PENDING
        {
            return false;
        }
        let key = raw.to_owned();
        let url = url.to_owned();
        self.entries.insert(key.clone(), Entry::Pending);
        let sender = self.sender.clone();
        let window = hwnd.0 as usize;
        std::thread::spawn(move || {
            let data = fetch(&url);
            let _ = sender.send((key, data));
            unsafe {
                let _ = PostMessageW(
                    Some(HWND(window as *mut _)),
                    WM_DIAGRAM_READY,
                    WPARAM(0),
                    LPARAM(0),
                );
            }
        });
        true
    }

    pub(super) fn receive(&mut self) -> (bool, bool) {
        let mut changed = false;
        let mut failed = false;
        while let Ok((url, data)) = self.receiver.try_recv() {
            if !matches!(self.entries.get(&url), Some(Entry::Pending)) {
                continue;
            }
            if let Some(data) = data {
                let mut size = self
                    .entries
                    .values()
                    .map(|entry| match entry {
                        Entry::Ready(bytes) => bytes.len(),
                        _ => 0,
                    })
                    .sum::<usize>();
                for entry in self.entries.values_mut() {
                    if size + data.len() <= CACHE_BYTES {
                        break;
                    }
                    if let Entry::Ready(bytes) = entry {
                        size -= bytes.len();
                        *entry = Entry::Failed;
                    }
                }
                self.entries.insert(url, Entry::Ready(data));
                changed = true;
            } else {
                self.entries.insert(url, Entry::Failed);
                failed = true;
            }
        }
        (changed, failed)
    }
}

fn fetch(url: &str) -> Option<Vec<u8>> {
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(6)))
        .max_redirects(3)
        .build();
    let agent = ureq::Agent::new_with_config(config);
    let mut response = agent.get(url).call().ok()?;
    let data = response
        .body_mut()
        .with_config()
        .limit(MAX_BYTES as u64)
        .read_to_vec()
        .ok()?;
    match ImageFormat::sniff(&data, false) {
        Some(format) if format.raster_dimensions_valid(&data) => Some(data),
        None => rasterize_svg(&data).map(|(png, _, _)| png),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
    };

    #[test]
    fn remote_image_fetch_accepts_an_image_and_rejects_html() {
        let image = include_bytes!("../../../assets/pane-md.png");
        for (body, valid) in [
            (image.as_slice(), true),
            (b"<html>not an image".as_slice(), false),
        ] {
            let server = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = server.local_addr().unwrap();
            let response = body.to_vec();
            let worker = std::thread::spawn(move || {
                let (mut stream, _) = server.accept().unwrap();
                let mut request = [0; 1024];
                let _ = stream.read(&mut request).unwrap();
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    response.len()
                )
                .unwrap();
                stream.write_all(&response).unwrap();
            });
            let result = fetch(&format!("http://{address}/image"));
            worker.join().unwrap();
            assert_eq!(result.is_some(), valid);
        }
    }
}
