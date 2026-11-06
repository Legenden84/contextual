//! The app's tokio runtime (plan §6.4). GPUI has its own executor; core code (reqwest, the
//! workspace host's blocking pool, child processes) needs tokio. Core futures run here and are
//! awaited from GPUI tasks through their `JoinHandle`.

use std::future::Future;

use gpui_kit::{App, Global};

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
