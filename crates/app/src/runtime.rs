//! The app's tokio runtime (plan §6.4). GPUI has its own executor; core code (reqwest, the
//! workspace host's blocking pool, child processes) needs tokio. Core futures run here and are
//! awaited from GPUI tasks through their `JoinHandle`.

use std::future::Future;
use std::time::Duration;

use contextual_core::llm::types::ChatEvent;
use futures::{Stream, StreamExt as _};
use gpui_kit::{App, Global};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// Bound the bridge as well as the Foundry client's own event queue.
pub const STREAM_QUEUE_CAPACITY: usize = 4;
const STREAM_UPDATE_INTERVAL: Duration = Duration::from_millis(33);
/// Flush large bursts early. This is a soft limit: one upstream delta can exceed it.
const STREAM_BATCH_BYTES: usize = 16 * 1024;

pub struct Runtime(tokio::runtime::Runtime);

impl Global for Runtime {}

pub fn init(cx: &mut App) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_name("contextual-core")
        .enable_all()
        .build()
        .expect("failed to start the tokio runtime");
    cx.set_global(Runtime(rt));
}

/// Runs `fut` on the tokio runtime. Await the result from a GPUI task; a panic in `fut` becomes
/// an internal error rather than taking the UI down.
pub fn spawn<T, F>(
    cx: &App,
    fut: F,
) -> impl Future<Output = contextual_core::Result<T>> + 'static + use<T, F>
where
    T: Send + 'static,
    F: Future<Output = contextual_core::Result<T>> + Send + 'static,
{
    let handle = cx.global::<Runtime>().0.spawn(fut);
    async move {
        handle.await.unwrap_or_else(|e| {
            Err(contextual_core::Error::internal(format!(
                "background task failed: {e}"
            )))
        })
    }
}

/// Forward a lossless, ordered stream to GPUI with fewer text updates. Run on Tokio.
/// Metadata and stream termination flush buffered text immediately; ordinary text waits
/// at most one batching interval when the UI keeps up. Backpressure never drops text.
///
/// Cancellation stops reading upstream and flushes text already accepted by this bridge.
/// A pending send is allowed to finish: the foreground consumer continues draining after
/// Stop. Dropping the UI receiver wakes pending sends and also ends an idle upstream stream.
pub async fn forward_chat_events(
    mut events: impl Stream<Item = ChatEvent> + Unpin,
    tx: mpsc::Sender<ChatEvent>,
    cancel: CancellationToken,
) {
    let mut pending = String::new();
    let mut received_deltas = 0usize;
    let mut received_bytes = 0usize;
    let mut published_batches = 0usize;
    let delay = tokio::time::sleep(STREAM_UPDATE_INTERVAL);
    tokio::pin!(delay);

    loop {
        tokio::select! {
            biased;
            _ = tx.closed() => break,
            _ = cancel.cancelled() => break,
            _ = &mut delay, if !pending.is_empty() => {
                if !flush_chat_text(&tx, &mut pending, &mut published_batches).await {
                    break;
                }
            }
            event = events.next() => {
                match event {
                    Some(ChatEvent::Delta { text }) => {
                        received_deltas += 1;
                        received_bytes += text.len();
                        if text.is_empty() {
                            continue;
                        }
                        if pending.is_empty() {
                            delay.as_mut().reset(
                                tokio::time::Instant::now() + STREAM_UPDATE_INTERVAL,
                            );
                        }
                        pending.push_str(&text);
                        if pending.len() >= STREAM_BATCH_BYTES
                            && !flush_chat_text(&tx, &mut pending, &mut published_batches).await
                        {
                            break;
                        }
                    }
                    Some(event) => {
                        if !flush_chat_text(&tx, &mut pending, &mut published_batches).await
                            || tx.send(event).await.is_err()
                        {
                            break;
                        }
                    }
                    None => break,
                }
            }
        }
    }

    // Drop the Foundry stream before awaiting the final UI send, so cancellation/receiver
    // closure cannot leave its HTTP task blocked on a full upstream channel.
    drop(events);
    flush_chat_text(&tx, &mut pending, &mut published_batches).await;
    tracing::debug!(
        target: "contextual::performance",
        received_deltas,
        received_bytes,
        published_batches,
        cancelled = cancel.is_cancelled(),
        "chat stream bridge finished"
    );
}

async fn flush_chat_text(
    tx: &mpsc::Sender<ChatEvent>,
    pending: &mut String,
    published: &mut usize,
) -> bool {
    if pending.is_empty() {
        return true;
    }
    let text = std::mem::take(pending);
    if tx.send(ChatEvent::Delta { text }).await.is_err() {
        return false;
    }
    *published += 1;
    true
}

static LOCAL_OFFSET: std::sync::OnceLock<time::UtcOffset> = std::sync::OnceLock::new();

/// Reads the local UTC offset. Call before any threads start: `time` refuses to read it later
/// on Unix (the libc call isn't thread-safe), and times would then show in UTC.
pub fn init_local_offset() {
    let _ =
        LOCAL_OFFSET.set(time::UtcOffset::current_local_offset().unwrap_or(time::UtcOffset::UTC));
}

/// Now, in local time.
pub fn now_local() -> time::OffsetDateTime {
    time::OffsetDateTime::now_utc()
        .to_offset(LOCAL_OFFSET.get().copied().unwrap_or(time::UtcOffset::UTC))
}

#[cfg(test)]
mod tests {
    use super::*;
    use contextual_core::llm::types::ApiKind;
    use futures::stream;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn delta(text: &str) -> ChatEvent {
        ChatEvent::Delta { text: text.into() }
    }

    async fn collect(events: Vec<ChatEvent>) -> Vec<ChatEvent> {
        let (tx, mut rx) = mpsc::channel(STREAM_QUEUE_CAPACITY);
        let job = tokio::spawn(forward_chat_events(
            stream::iter(events),
            tx,
            CancellationToken::new(),
        ));
        let mut out = Vec::new();
        while let Some(event) = rx.recv().await {
            out.push(event);
        }
        job.await.unwrap();
        out
    }

    #[tokio::test(start_paused = true)]
    async fn bursts_preserve_unicode_and_reduce_updates() {
        let piece = "café ☕\n";
        let count = 10_000;
        let out = collect((0..count).map(|_| delta(piece)).collect()).await;
        let text: String = out
            .iter()
            .map(|event| match event {
                ChatEvent::Delta { text } => text.as_str(),
                _ => panic!("unexpected metadata"),
            })
            .collect();
        assert_eq!(text, piece.repeat(count));
        assert!(out.len() < 10, "{} updates", out.len());
    }

    #[tokio::test(start_paused = true)]
    async fn metadata_and_eof_flush_text_in_order() {
        let usage = ChatEvent::Usage {
            input_tokens: 10,
            output_tokens: 5,
            cached_tokens: 0,
        };
        let done = ChatEvent::Done {
            finish_reason: Some("stop".into()),
            response_id: Some("response".into()),
            api: ApiKind::Responses,
            chain_reset: false,
        };
        assert_eq!(
            collect(vec![
                delta("one"),
                delta(" two"),
                usage.clone(),
                delta(" three"),
                done.clone(),
            ])
            .await,
            vec![delta("one two"), usage, delta(" three"), done],
        );
        assert_eq!(
            collect(vec![delta(""), delta("unfinished")]).await,
            vec![delta("unfinished")],
        );
        assert!(collect(vec![delta("")]).await.is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn errors_do_not_discard_the_text_before_them() {
        let error = ChatEvent::Error {
            message: "connection interrupted".into(),
        };
        assert_eq!(
            collect(vec![delta("partial "), delta("reply"), error.clone()]).await,
            vec![delta("partial reply"), error],
        );
    }

    #[tokio::test(start_paused = true)]
    async fn timer_publishes_text_without_waiting_for_another_event() {
        let (source, events) = futures::channel::mpsc::unbounded();
        source.unbounded_send(delta("first")).unwrap();
        let (tx, mut rx) = mpsc::channel(STREAM_QUEUE_CAPACITY);
        let started = tokio::time::Instant::now();
        let job = tokio::spawn(forward_chat_events(events, tx, CancellationToken::new()));

        // Tokio's paused clock advances to the timer when no other task can progress.
        assert_eq!(rx.recv().await, Some(delta("first")));
        assert_eq!(started.elapsed(), STREAM_UPDATE_INTERVAL);

        source.unbounded_send(delta("second")).unwrap();
        assert_eq!(rx.recv().await, Some(delta("second")));
        assert_eq!(started.elapsed(), STREAM_UPDATE_INTERVAL * 2);
        drop(source);
        assert_eq!(rx.recv().await, None);
        job.await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn cancellation_flushes_accepted_text_without_waiting_for_the_timer() {
        let events = stream::iter(vec![delta("accepted")]).chain(stream::pending());
        let cancel = CancellationToken::new();
        let (tx, mut rx) = mpsc::channel(STREAM_QUEUE_CAPACITY);
        let started = tokio::time::Instant::now();
        let job = tokio::spawn(forward_chat_events(events, tx, cancel.clone()));
        tokio::task::yield_now().await;
        cancel.cancel();

        assert_eq!(rx.recv().await, Some(delta("accepted")));
        assert_eq!(rx.recv().await, None);
        job.await.unwrap();
        assert_eq!(started.elapsed(), Duration::ZERO);
    }

    #[tokio::test(start_paused = true)]
    async fn slow_ui_bounds_upstream_consumption_and_stop_can_drain() {
        let consumed = Arc::new(AtomicUsize::new(0));
        let counter = consumed.clone();
        let piece = "x".repeat(STREAM_BATCH_BYTES);
        let events = stream::iter(0..100).map(move |_| {
            counter.fetch_add(1, Ordering::SeqCst);
            delta(&piece)
        });
        let cancel = CancellationToken::new();
        let (tx, mut rx) = mpsc::channel(STREAM_QUEUE_CAPACITY);
        let job = tokio::spawn(forward_chat_events(events, tx, cancel.clone()));
        tokio::task::yield_now().await;

        // Four queued batches and one waiting to send; not the entire response.
        assert_eq!(consumed.load(Ordering::SeqCst), STREAM_QUEUE_CAPACITY + 1);
        cancel.cancel();
        let mut drained = 0;
        while let Some(event) = rx.recv().await {
            assert!(matches!(event, ChatEvent::Delta { .. }));
            drained += 1;
        }
        job.await.unwrap();
        assert_eq!(drained, STREAM_QUEUE_CAPACITY + 1);
        assert_eq!(consumed.load(Ordering::SeqCst), drained);
    }

    #[tokio::test(start_paused = true)]
    async fn dropping_the_ui_receiver_ends_an_idle_stream() {
        let (tx, rx) = mpsc::channel(STREAM_QUEUE_CAPACITY);
        let job = tokio::spawn(forward_chat_events(
            stream::pending::<ChatEvent>(),
            tx,
            CancellationToken::new(),
        ));
        tokio::task::yield_now().await;
        drop(rx);
        tokio::time::timeout(Duration::from_secs(1), job)
            .await
            .expect("bridge kept waiting on upstream")
            .unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn dropping_the_ui_receiver_releases_a_blocked_send() {
        let (tx, rx) = mpsc::channel(STREAM_QUEUE_CAPACITY);
        let events = stream::iter((0..100).map(|_| delta(&"x".repeat(STREAM_BATCH_BYTES))));
        let job = tokio::spawn(forward_chat_events(events, tx, CancellationToken::new()));
        tokio::task::yield_now().await;
        drop(rx);
        tokio::time::timeout(Duration::from_secs(1), job)
            .await
            .expect("bridge kept waiting on a closed UI queue")
            .unwrap();
    }
}
