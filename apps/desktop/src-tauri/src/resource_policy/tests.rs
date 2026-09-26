use super::*;
use crate::model_runtime::RuntimeCancellation;
use std::time::Duration;

async fn wait_for_queue(controller: &AdmissionController, wanted: usize) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while controller.snapshot().queued_work != wanted {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("expected waiters to enter the admission queue");
}

#[tokio::test]
async fn playback_blocks_background_but_keeps_interactive_work_available() {
    let controller = Arc::new(AdmissionController::default());
    let playback = controller.playback_started();
    let cancel = RuntimeCancellation::default();
    let background_controller = controller.clone();
    let background = tokio::spawn(async move {
        background_controller
            .admit(WorkClass::Background, &cancel)
            .await
    });
    wait_for_queue(&controller, 1).await;

    let interactive = tokio::time::timeout(
        Duration::from_secs(2),
        controller.admit(WorkClass::Interactive, &RuntimeCancellation::default()),
    )
    .await
    .expect("interactive search must remain available during playback")
    .unwrap();
    assert_eq!(
        controller.snapshot().active_work_class,
        Some(WorkClass::Interactive)
    );
    drop(interactive);
    assert!(!background.is_finished());

    drop(playback);
    let permit = tokio::time::timeout(Duration::from_secs(2), background)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(
        controller.snapshot().active_work_class,
        Some(WorkClass::Background)
    );
    drop(permit);
}

#[tokio::test]
async fn higher_priority_waiter_runs_before_background_work() {
    let controller = Arc::new(AdmissionController::default());
    let held = controller
        .admit(WorkClass::Maintenance, &RuntimeCancellation::default())
        .await
        .unwrap();

    let background_controller = controller.clone();
    let background = tokio::spawn(async move {
        background_controller
            .admit(WorkClass::Background, &RuntimeCancellation::default())
            .await
    });
    wait_for_queue(&controller, 1).await;

    let interactive_controller = controller.clone();
    let interactive = tokio::spawn(async move {
        interactive_controller
            .admit(WorkClass::Interactive, &RuntimeCancellation::default())
            .await
    });
    wait_for_queue(&controller, 2).await;
    drop(held);

    let interactive_permit = tokio::time::timeout(Duration::from_secs(2), interactive)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(
        controller.snapshot().active_work_class,
        Some(WorkClass::Interactive)
    );
    drop(interactive_permit);
    let background_permit = tokio::time::timeout(Duration::from_secs(2), background)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(
        controller.snapshot().active_work_class,
        Some(WorkClass::Background)
    );
    drop(background_permit);
}

#[tokio::test]
async fn waiters_with_equal_priority_are_admitted_fifo() {
    let controller = Arc::new(AdmissionController::default());
    let held = controller
        .admit(WorkClass::Maintenance, &RuntimeCancellation::default())
        .await
        .unwrap();
    let (first_started, first_rx) = tokio::sync::oneshot::channel();
    let (first_release, first_wait) = tokio::sync::oneshot::channel();
    let first_controller = controller.clone();
    let first = tokio::spawn(async move {
        let permit = first_controller
            .admit(WorkClass::Foreground, &RuntimeCancellation::default())
            .await
            .unwrap();
        first_started.send(()).unwrap();
        let _ = first_wait.await;
        drop(permit);
    });
    wait_for_queue(&controller, 1).await;

    let (second_started, mut second_rx) = tokio::sync::oneshot::channel();
    let second_controller = controller.clone();
    let second = tokio::spawn(async move {
        let _permit = second_controller
            .admit(WorkClass::Foreground, &RuntimeCancellation::default())
            .await
            .unwrap();
        second_started.send(()).unwrap();
    });
    wait_for_queue(&controller, 2).await;
    drop(held);

    tokio::time::timeout(Duration::from_secs(2), first_rx)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        second_rx.try_recv(),
        Err(tokio::sync::oneshot::error::TryRecvError::Empty)
    ));
    first_release.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(2), second_rx)
        .await
        .unwrap()
        .unwrap();
    first.await.unwrap();
    second.await.unwrap();
}

#[tokio::test]
async fn cancelled_waiter_is_removed_and_playback_end_resumes_queue() {
    let controller = Arc::new(AdmissionController::default());
    let playback = controller.playback_started();
    let cancellation = RuntimeCancellation::default();
    let token = cancellation.clone();
    let waiting_controller = controller.clone();
    let waiting =
        tokio::spawn(async move { waiting_controller.admit(WorkClass::Metadata, &token).await });
    wait_for_queue(&controller, 1).await;
    cancellation.cancel();
    assert!(waiting.await.unwrap().is_err());
    assert_eq!(controller.snapshot().queued_work, 0);

    let resume_controller = controller.clone();
    let resume = tokio::spawn(async move {
        resume_controller
            .admit(WorkClass::Maintenance, &RuntimeCancellation::default())
            .await
    });
    wait_for_queue(&controller, 1).await;
    drop(playback);
    let permit = tokio::time::timeout(Duration::from_secs(2), resume)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(controller.snapshot().playback_sessions, 0);
    drop(permit);
}

#[tokio::test]
async fn active_playback_guards_are_reference_counted() {
    let controller = Arc::new(AdmissionController::default());
    let first = controller.playback_started();
    let second = controller.playback_started();
    assert_eq!(controller.snapshot().playback_sessions, 2);
    drop(first);
    assert_eq!(controller.snapshot().playback_sessions, 1);
    drop(second);
    assert_eq!(controller.snapshot().playback_sessions, 0);
}
