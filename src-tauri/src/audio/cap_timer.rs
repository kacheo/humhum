//! Absolute recording safety timer. Stops a capture after five minutes if the
//! normal hotkey stop is missed. No upgrade or warning events are emitted.

use std::time::Duration;
use tokio::sync::{mpsc, oneshot};

pub enum CapEvent {
    Stop,
}

pub async fn safety_cap_timer(
    stop_after: Duration,
    cap_tx: mpsc::Sender<CapEvent>,
    cancel_rx: oneshot::Receiver<()>,
) {
    tokio::select! {
        biased;
        _ = cancel_rx => {} // normal stop/cancel before the cap — fire nothing
        _ = tokio::time::sleep(stop_after) => {
            if let Err(e) = cap_tx.send(CapEvent::Stop).await {
                log::warn!("safety_cap_timer: Stop send failed (receiver dropped?): {e}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::{advance, Duration};

    const SAFETY: Duration = Duration::from_secs(300);

    #[tokio::test(start_paused = true)]
    async fn safety_cap_sends_stop_at_stop_after() {
        let (tx, mut rx) = mpsc::channel(1);
        let (_cancel_tx, cancel_rx) = oneshot::channel();
        tokio::spawn(safety_cap_timer(SAFETY, tx, cancel_rx));

        // Just before the cap: nothing.
        advance(SAFETY - Duration::from_millis(1)).await;
        assert!(rx.try_recv().is_err(), "safety stop fired early");

        // Crossing the cap: a single Stop, no warning/reached emits to assert (none).
        advance(Duration::from_millis(1)).await;
        assert!(matches!(rx.recv().await, Some(CapEvent::Stop)));
    }

    #[tokio::test(start_paused = true)]
    async fn safety_cap_cancel_suppresses_stop() {
        let (tx, mut rx) = mpsc::channel(1);
        let (cancel_tx, cancel_rx) = oneshot::channel();
        let handle = tokio::spawn(safety_cap_timer(SAFETY, tx, cancel_rx));

        advance(Duration::from_secs(30)).await;
        drop(cancel_tx); // normal stop/cancel before the safety cap
        handle.await.unwrap();

        advance(SAFETY).await;
        assert!(rx.try_recv().is_err(), "stop fired after cancel");
    }

    #[tokio::test(start_paused = true)]
    async fn safety_cap_timer_survives_a_dropped_receiver() {
        let (tx, rx) = mpsc::channel(1);
        drop(rx);
        let (_cancel_tx, cancel_rx) = oneshot::channel();
        let handle = tokio::spawn(safety_cap_timer(SAFETY, tx, cancel_rx));

        advance(SAFETY).await;
        handle
            .await
            .expect("safety timer task must not panic when the receiver is dropped");
    }
}
