//! Speculative first frames use a separate thread and a replaceable two-job
//! mailbox. They cannot occupy the interactive renderer or fetch resources.
use super::*;
use futures::StreamExt;
use tokio::sync::watch;

#[derive(Debug, Clone, PartialEq)]
pub struct Key {
    pub id: String,
    pub signature: [u8; 32],
    pub font_size: u16,
    pub hide_quotes: bool,
    pub allow_images: bool,
    pub image_revision: u64,
    pub viewport: Viewport,
}
#[derive(Debug, Clone)]
pub struct Request {
    pub key: Key,
    pub source: Source,
}
#[derive(Debug, Clone)]
pub enum Event {
    Ready(watch::Sender<Vec<Request>>),
    Prepared(Key, Option<Arc<Frame>>),
}
pub fn subscription() -> impl futures::Stream<Item = Event> {
    iced::stream::channel(2, |mut output: mpsc::Sender<Event>| async move {
        let (tx, rx) = watch::channel(Vec::new());
        if output.send(Event::Ready(tx)).await.is_err() {
            return;
        }
        let _ = tokio::task::spawn_blocking(move || worker(rx, output)).await;
    })
}
fn worker(mut input: watch::Receiver<Vec<Request>>, mut output: mpsc::Sender<Event>) {
    let mut font_system = None;
    while futures::executor::block_on(input.changed()).is_ok() {
        let requests = input.borrow_and_update().clone();
        for request in requests.into_iter().take(2) {
            if input.has_changed().unwrap_or(true) {
                break;
            }
            // A one-frame session uses the same rendering path, then releases
            // its DOM, glyph cache and decoded resources before the next job.
            let (tx, mut commands) = commands::channel(1);
            let (mut frames, mut rx) = mpsc::channel(1);
            let _ = tx.try_send(Input::Clear);
            let result = document(
                0,
                request.source,
                request.key.viewport,
                font_system.get_or_insert_with(fonts).clone(),
                None,
                &mut commands,
                &mut frames,
            );
            drop(frames);
            let frame = if result.is_ok() {
                match futures::executor::block_on(rx.next()) {
                    Some(super::Event::Frame(frame)) => Some(frame),
                    _ => None,
                }
            } else {
                None
            };
            if futures::executor::block_on(output.send(Event::Prepared(request.key, frame)))
                .is_err()
            {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn speculative_mailbox_replaces_obsolete_work_and_seeds_permitted_images() {
        let viewport = Viewport {
            width: 320,
            height: 200,
            scale: 1.5,
        };
        let body = Arc::new(HtmlBody::new(
            "<p>Current neighbor</p><img src='https://example.test/logo.webp' width=60 height=60>"
                .into(),
            Default::default(),
        ));
        let request = Request {
            key: Key {
                id: "current".into(),
                signature: body.signature,
                font_size: 14,
                hide_quotes: true,
                allow_images: true,
                image_revision: 1,
                viewport,
            },
            source: Source {
                body,
                font_size: 14,
                hide_quotes: true,
                images: vec![(
                    "https://example.test/logo.webp".into(),
                    Arc::from(include_bytes!("../../assets/logo-light.webp").as_slice()),
                )],
            },
        };
        let (tx, rx) = watch::channel(Vec::new());
        tx.send_replace(vec![Request {
            key: Key {
                id: "obsolete".into(),
                ..request.key.clone()
            },
            ..request.clone()
        }]);
        tx.send_replace(vec![request.clone()]);
        let (output, mut events) = mpsc::channel(2);
        let thread = std::thread::spawn(move || worker(rx, output));
        let result = tokio::time::timeout(std::time::Duration::from_secs(20), events.next())
            .await
            .unwrap();
        let Some(Event::Prepared(key, Some(frame))) = result else {
            panic!("Expected a prepared initial frame")
        };
        assert_eq!(key.id, "current");
        assert!(frame.matches_view(viewport, 0.));
        assert_eq!(frame.pixels.len(), 480 * 300 * 4);
        assert_eq!(
            frame.images,
            ["https://example.test/logo.webp"],
            "Only resources actually used by the document are reported"
        );
        drop(tx);
        thread.join().unwrap();
    }
}
