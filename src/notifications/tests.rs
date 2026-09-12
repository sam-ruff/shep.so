use super::*;
use futures::StreamExt;
use tokio::sync::{mpsc, oneshot};

fn state(total: u64) -> State {
    State {
        total,
        latest: Some(Arc::new(Arrival {
            account: "fixture".into(),
            message: total.to_string(),
            sender: "Fixture sender".into(),
            subject: "A private subject".into(),
        })),
        ..State::default()
    }
}

#[test]
fn notification_policy_keeps_sound_popup_and_privacy_independent() {
    for popups in [false, true] {
        for sound in [false, true] {
            for details in [false, true] {
                let mut request = state(1);
                request.settings = Settings {
                    popups,
                    sound,
                    show_details: details,
                };
                let result = Delivery::prepare(&request, 1);
                assert_eq!(result.is_some(), popups || sound);
                if let Some(result) = result {
                    assert_eq!((result.popups, result.sound), (popups, sound));
                    assert_eq!(result.body.contains("private"), details);
                    assert_eq!(result.title.contains("Fixture"), details);
                }
                assert!(Delivery::prepare(&request, 0).is_none());
            }
        }
    }
    let group = Delivery::prepare(&state(70), 70).unwrap();
    assert_eq!(group.title, "70 new emails");
    assert!(!group.body.contains("private"));
}

#[tokio::test(start_paused = true)]
async fn burst_samples_latest_privacy_and_never_replays_muted_arrivals() {
    let (tx, rx) = watch::channel(State::default());
    let (output, mut events) = futures::channel::mpsc::channel(8);
    let worker = tokio::spawn(drive(rx, mpsc::channel(1).1, output, |_| async { Ok(()) }));
    tx.send_replace(state(1));
    tokio::task::yield_now().await;
    let mut changed = state(1);
    changed.settings.show_details = false;
    tx.send_replace(changed);
    let Event::Sent(sent) = events.next().await.unwrap() else {
        panic!("expected delivery")
    };
    assert_eq!(sent.count, 1);
    assert_eq!(sent.title, "New email");
    for total in 2..=500 {
        let mut request = state(total);
        request.settings.popups = false;
        request.settings.sound = false;
        tx.send_replace(request);
    }
    assert!(matches!(events.next().await, Some(Event::Skipped(500))));
    tx.send_replace(state(500)); // Re-enabling alone cannot replay 499 arrivals.
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_millis(200)).await;
    assert!(events.try_recv().is_err());
    tx.send_replace(state(501));
    let Event::Sent(sent) = events.next().await.unwrap() else {
        panic!("expected fresh delivery")
    };
    assert_eq!((sent.count, sent.through), (1, 501));
    drop(tx);
    worker.await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn blocked_desktop_coalesces_all_new_arrivals_then_recovers_after_failure() {
    let (tx, rx) = watch::channel(State::default());
    let (output, mut events) = futures::channel::mpsc::channel(8);
    let (calls, mut pending) = mpsc::channel::<(Delivery, oneshot::Sender<anyhow::Result<()>>)>(1);
    let worker = tokio::spawn(drive(rx, mpsc::channel(1).1, output, move |delivery| {
        let calls = calls.clone();
        async move {
            let (ack, result) = oneshot::channel();
            calls.send((delivery, ack)).await.unwrap();
            result.await.unwrap()
        }
    }));
    tx.send_replace(state(1));
    let (first, ack) = pending.recv().await.unwrap();
    assert_eq!(first.count, 1);
    for total in 2..=1000 {
        tx.send_replace(state(total));
    }
    assert!(pending.try_recv().is_err()); // Exactly one native request in flight.
    ack.send(Err(anyhow::anyhow!("fixture service unavailable")))
        .unwrap();
    assert!(matches!(events.next().await, Some(Event::Failed(d, error))
        if d.through == 1 && error.contains("service unavailable")));
    let (next, ack) = pending.recv().await.unwrap();
    assert_eq!((next.count, next.through), (999, 1000));
    ack.send(Ok(())).unwrap();
    assert!(matches!(events.next().await, Some(Event::Sent(d)) if d.count == 999));
    drop(tx);
    worker.await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn saving_receipts_share_the_worker_without_consuming_new_mail_counts() -> anyhow::Result<()>
{
    let (tx, rx) = watch::channel(State::default());
    let (system, requests) = mpsc::channel(1);
    let (output, mut events) = futures::channel::mpsc::channel(8);
    let mut calls = 0;
    let worker = tokio::spawn(drive(rx, requests, output, move |delivery: Delivery| {
        calls += 1;
        let call = calls;
        async move {
            if call == 1 {
                assert_eq!(delivery.through, 0);
                anyhow::bail!("Saving notification rejected");
            }
            assert_eq!(delivery.through, 1);
            Ok(())
        }
    }));
    let (_cancel, cancellation) = watch::channel(());
    let error = SystemSender(system)
        .saving_notification(cancellation)
        .await
        .expect_err("Expected native rejection");
    assert!(error.contains("Saving notification rejected"));
    assert!(events.try_recv().is_err());
    tx.send_replace(state(1));
    assert!(matches!(events.next().await, Some(Event::Sent(d)) if d.count == 1 && d.through == 1));
    drop(tx);
    worker.await?;
    Ok(())
}

#[tokio::test]
async fn saving_request_queue_is_bounded_and_shutdown_reports_failure() -> anyhow::Result<()> {
    let (system, requests) = mpsc::channel(1);
    let sender = SystemSender(system);
    let (_cancel, cancellation) = watch::channel(());
    let first = tokio::spawn(sender.clone().saving_notification(cancellation.clone()));
    tokio::task::yield_now().await;
    let second = tokio::spawn(sender.saving_notification(cancellation));
    tokio::task::yield_now().await;
    assert_eq!(requests.len(), 1);
    assert!(
        !second.is_finished(),
        "A newer close waits for cancelled queue membership to drain"
    );
    drop(requests);
    assert!(first.await?.is_err());
    assert!(second.await?.is_err());
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn abandoned_saving_request_is_not_delivered_after_its_caller_leaves() -> anyhow::Result<()> {
    let (tx, rx) = watch::channel(State::default());
    let (system, requests) = mpsc::channel(1);
    let (output, mut events) = futures::channel::mpsc::channel(8);
    let (receipt, result) = oneshot::channel();
    let (_cancel, cancellation) = watch::channel(());
    system
        .send(SystemRequest {
            delivery: saving_delivery(),
            receipt,
            cancellation,
        })
        .await?;
    drop(result);
    let worker = tokio::spawn(drive(
        rx,
        requests,
        output,
        |delivery: Delivery| async move {
            assert_ne!(delivery.through, 0, "Abandoned saving notice was delivered");
            Ok(())
        },
    ));
    tx.send_replace(state(1));
    assert!(matches!(events.next().await, Some(Event::Sent(_))));
    drop(tx);
    worker.await?;
    Ok(())
}
