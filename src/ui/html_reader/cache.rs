use super::*;
use crate::html_render::preparation::{Key, Request};

const MAX_FRAMES: usize = 8;
const MAX_BYTES: usize = 32 * 1024 * 1024;

#[derive(Default)]
pub(in crate::ui) struct Cache {
    frames: VecDeque<(Key, Arc<html_render::Frame>, widget::image::Handle)>,
    pub tx: Option<tokio::sync::watch::Sender<Vec<Request>>>,
    scheduled: Vec<Key>,
    pub hits: u64,
}
impl Cache {
    pub fn insert(&mut self, key: Key, frame: Arc<html_render::Frame>) {
        // Adjacent preparation must not replace a visited image-complete frame.
        if self.frames.iter().any(|(old, ..)| old == &key) {
            return;
        }
        let handle = handle(&frame);
        self.remember(key, frame, handle);
    }
    pub fn remember(
        &mut self,
        key: Key,
        frame: Arc<html_render::Frame>,
        handle: widget::image::Handle,
    ) {
        if frame_bytes(&frame) > MAX_BYTES || frame.scroll != 0. || frame.pan != 0. {
            return;
        }
        self.frames.retain(|(old, ..)| old != &key);
        self.frames.push_front((key, frame, handle));
        while self.frames.len() > MAX_FRAMES || self.bytes() > MAX_BYTES {
            self.frames.pop_back();
        }
    }
    pub fn get(
        &mut self,
        key: &Key,
        generation: u64,
    ) -> Option<(Arc<html_render::Frame>, widget::image::Handle)> {
        let index = self.frames.iter().position(|(old, ..)| old == key)?;
        let item = self.frames.remove(index)?;
        let frame = Arc::new(html_render::Frame {
            generation,
            ..(*item.1).clone()
        });
        let handle = item.2.clone();
        self.frames.push_front(item);
        self.hits += 1;
        Some((frame, handle))
    }
    pub fn image_arrived(&mut self, url: &str) {
        self.frames.retain(|(key, frame, _)| {
            !key.allow_images || !frame.images.iter().any(|used| used == url)
        });
        self.scheduled.clear();
    }
    fn images(&self, key: &Key) -> Vec<(String, Arc<[u8]>)> {
        self.frames
            .iter()
            .find(|(old, ..)| old == key)
            .map(|(_, frame, _)| frame.loaded_images.clone())
            .unwrap_or_default()
    }
    pub fn bytes(&self) -> usize {
        self.frames
            .iter()
            .map(|(_, frame, _)| frame_bytes(frame))
            .sum()
    }
    pub fn ids(&self) -> Vec<&str> {
        self.frames
            .iter()
            .map(|(key, ..)| key.id.as_str())
            .collect()
    }
}
fn frame_bytes(frame: &html_render::Frame) -> usize {
    frame.pixels.len()
        + frame
            .loaded_images
            .iter()
            .map(|(url, bytes)| url.len() + bytes.len())
            .sum::<usize>()
}

pub(super) fn handle(frame: &html_render::Frame) -> widget::image::Handle {
    widget::image::Handle::from_rgba(
        frame.width,
        frame.height,
        bytes::Bytes::from_owner(frame.pixels.clone()),
    )
}
impl App {
    pub(super) fn html_preparation(
        &self,
        detail: &MailDetail,
        viewport: Viewport,
    ) -> Option<Request> {
        let body = detail.html.as_ref()?;
        let allow_images = crate::remote_images::allowed(&self.preferences, &detail.summary);
        let key = Key {
            id: detail.summary.id.clone(),
            signature: body.signature,
            font_size: self.preferences.reader_font_size,
            hide_quotes: self.html_quotes_hidden(detail),
            allow_images,
            viewport,
        };
        // Keep only this document's images. A visited frame owns its exact
        // decoded inputs even when the shared download cache evicts them.
        let mut images = if allow_images {
            self.html_reader.cache.images(&key)
        } else {
            Vec::new()
        };
        if allow_images {
            for (url, bytes) in &self.remote_bytes {
                if body.remote_images.iter().any(|image| image.url == *url)
                    || images.iter().any(|(used, _)| used == url)
                {
                    images.retain(|(used, _)| used != url);
                    images.push((url.clone(), bytes.clone()));
                }
            }
        }
        Some(Request {
            source: html_render::Source {
                body: body.clone(),
                font_size: key.font_size,
                hide_quotes: key.hide_quotes,
                images,
            },
            key,
        })
    }
    pub(super) fn html_frame_cacheable(&self, key: &Key, frame: &html_render::Frame) -> bool {
        if !key.allow_images {
            return true;
        }
        // Pending or failed downloads do not prevent retaining the text/layout.
        // But a frame cannot claim a download that the worker has not decoded.
        // Each successful arrival invalidates only frames that use that URL.
        frame
            .images
            .iter()
            .filter(|url| remote_url(url))
            .all(|url| {
                self.remote_bytes
                    .iter()
                    .find(|(used, _)| used == url)
                    .is_none_or(|(_, latest)| {
                        frame
                            .loaded_images
                            .iter()
                            .find(|(used, _)| used == url)
                            .is_some_and(|(_, applied)| Arc::ptr_eq(applied, latest))
                    })
            })
    }
    pub(super) fn preload_html(&mut self) {
        let requests = self
            .html_reader
            .viewport
            .filter(|_| self.html_reader.key.is_some())
            .map(|(viewport, _)| {
                let rows = if self.conversation_visible() {
                    &self.conversation.page.rows
                } else {
                    &self.page.rows
                };
                let index = rows
                    .iter()
                    .position(|row| Some(row.id.as_str()) == self.reader_id());
                index
                    .into_iter()
                    .flat_map(|i| [Some(i + 1), i.checked_sub(1)])
                    .flatten()
                    .filter_map(|i| rows.get(i))
                    .filter_map(|row| self.detail_cache.iter().find(|d| d.summary.id == row.id))
                    .filter_map(|detail| self.html_preparation(detail, viewport))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let cache = &mut self.html_reader.cache;
        let keys: Vec<_> = requests.iter().map(|request| request.key.clone()).collect();
        if cache.scheduled != keys
            && let Some(tx) = &cache.tx
        {
            let missing = requests
                .into_iter()
                .filter(|request| !cache.frames.iter().any(|(key, ..)| key == &request.key))
                .collect();
            cache.scheduled = keys;
            tx.send_replace(missing);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn frame_cache_bounds_memory_evicts_lru_and_rejects_noninitial_frames() {
        let size = Viewport {
            width: 1600,
            height: 1600,
            scale: 1.,
        };
        let key = Key {
            id: "0".into(),
            signature: [1; 32],
            font_size: 14,
            hide_quotes: true,
            allow_images: false,
            viewport: size,
        };
        let frame = Arc::new(html_render::Frame {
            generation: 0,
            layout_revision: 1,
            viewport: size,
            pixels: Arc::from(vec![0; 1600 * 1600 * 4]),
            width: 1600,
            height: 1600,
            content_height: 2000.,
            content_width: 1600.,
            pan: 0.,
            scroll: 0.,
            images: vec![],
            loaded_images: vec![],
            background: None,
            reflow: None,
        });
        let mut cache = Cache::default();
        for i in 0..3 {
            cache.insert(
                Key {
                    id: i.to_string(),
                    ..key.clone()
                },
                frame.clone(),
            );
        }
        assert!(cache.get(&key, 4).is_some());
        cache.insert(
            Key {
                id: "3".into(),
                ..key.clone()
            },
            frame.clone(),
        );
        assert_eq!(cache.ids(), ["3", "0", "2"]);
        assert!(cache.bytes() <= MAX_BYTES);
        cache.insert(
            Key {
                id: "scrolled".into(),
                ..key
            },
            Arc::new(html_render::Frame {
                scroll: 400.,
                ..(*frame).clone()
            }),
        );
        assert_eq!(cache.ids(), ["3", "0", "2"]);
    }
}
