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
            .admit(WorkClass::Explicit, &RuntimeCancellation::default())
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
            .admit(WorkClass::Explicit, &RuntimeCancellation::default())
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

const DEADLINE: Duration = Duration::from_secs(5);

async fn until(controller: &AdmissionController, check: impl Fn(AdmissionSnapshot) -> bool) {
    tokio::time::timeout(DEADLINE, async {
        while !check(controller.snapshot()) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("governor state not reached");
}

type Granted = tokio::sync::mpsc::UnboundedSender<(WorkClass, AdmissionPermit)>;

fn spawn_model_waiter(controller: &Arc<AdmissionController>, class: WorkClass, granted: &Granted) {
    let controller = Arc::clone(controller);
    let granted = granted.clone();
    tokio::spawn(async move {
        let permit = controller
            .admit(class, &RuntimeCancellation::default())
            .await
            .unwrap();
        let _ = granted.send((class, permit));
    });
}

#[test]
fn priority_tiers_follow_the_master_plan_order() {
    let tiers: Vec<u8> = WorkClass::ALL.iter().map(|c| c.tier()).collect();
    assert_eq!(tiers, [2, 3, 4, 5, 6, 7], "tier 1 is playback itself");
    let during_playback: Vec<_> = WorkClass::ALL
        .into_iter()
        .filter(|c| c.allowed_during_playback())
        .collect();
    assert_eq!(
        during_playback,
        [
            WorkClass::Realtime,
            WorkClass::Interactive,
            WorkClass::Explicit
        ]
    );
    let may_exceed: Vec<_> = WorkClass::ALL
        .into_iter()
        .filter(|c| c.may_exceed_residency())
        .collect();
    assert_eq!(may_exceed, [WorkClass::Realtime, WorkClass::Interactive]);
}

#[tokio::test]
async fn queued_work_is_granted_in_strict_priority_order() {
    let controller = Arc::new(AdmissionController::default());
    let held = controller
        .admit(WorkClass::Maintenance, &RuntimeCancellation::default())
        .await
        .unwrap();
    let (granted, mut grants) = tokio::sync::mpsc::unbounded_channel();
    // Enqueue lowest priority first so FIFO alone would produce the reverse.
    for (queued, class) in WorkClass::ALL.into_iter().rev().enumerate() {
        spawn_model_waiter(&controller, class, &granted);
        wait_for_queue(&controller, queued + 1).await;
    }
    drop(held);
    let mut order = Vec::new();
    for _ in WorkClass::ALL {
        let (class, permit) = tokio::time::timeout(DEADLINE, grants.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(controller.snapshot().active_work_class, Some(class));
        order.push(class);
        drop(permit);
    }
    assert_eq!(order, WorkClass::ALL);
}

#[tokio::test]
async fn aging_bounds_starvation_without_overtaking_realtime() {
    let controller = Arc::new(AdmissionController::default());
    let (granted, mut grants) = tokio::sync::mpsc::unbounded_channel();
    let mut permit = controller
        .admit(WorkClass::Interactive, &RuntimeCancellation::default())
        .await
        .unwrap();
    spawn_model_waiter(&controller, WorkClass::Maintenance, &granted);
    wait_for_queue(&controller, 1).await;

    // A continuous stream of interactive work: one new request per grant.
    let mut interactive_grants = 0;
    loop {
        spawn_model_waiter(&controller, WorkClass::Interactive, &granted);
        wait_for_queue(&controller, 2).await;
        if interactive_grants == AGING_LIMIT {
            // Maintenance is now aged; realtime must still come first.
            spawn_model_waiter(&controller, WorkClass::Realtime, &granted);
            wait_for_queue(&controller, 3).await;
        }
        drop(permit);
        let (class, next) = tokio::time::timeout(DEADLINE, grants.recv())
            .await
            .unwrap()
            .unwrap();
        permit = next;
        match class {
            WorkClass::Interactive => interactive_grants += 1,
            WorkClass::Realtime => {
                assert_eq!(interactive_grants, AGING_LIMIT);
                drop(permit);
                let (class, next) = tokio::time::timeout(DEADLINE, grants.recv())
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(class, WorkClass::Maintenance, "aged waiter is next");
                permit = next;
                break;
            }
            other => panic!("unexpected grant before realtime: {other:?}"),
        }
        assert!(interactive_grants <= AGING_LIMIT, "maintenance starved");
    }
    drop(permit);
    // The remaining interactive waiter still runs: nothing is lost.
    let (class, last) = tokio::time::timeout(DEADLINE, grants.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(class, WorkClass::Interactive);
    drop(last);
    until(&controller, |s| {
        s.queued_work == 0 && s.active_work_class.is_none()
    })
    .await;
}

#[tokio::test]
async fn job_lane_is_bounded_and_independent_of_the_model_lane() {
    let controller = Arc::new(AdmissionController::new(ResourceBudget::DEFAULT));
    let cancel = RuntimeCancellation::default();
    let first = controller
        .acquire_job_slot(WorkClass::Explicit, &cancel)
        .await
        .unwrap();
    let second = controller
        .acquire_job_slot(WorkClass::Metadata, &cancel)
        .await
        .unwrap();
    // A running job never blocks model inference.
    let model = tokio::time::timeout(DEADLINE, controller.admit(WorkClass::Background, &cancel))
        .await
        .expect("model lane is separate")
        .unwrap();
    let third_controller = Arc::clone(&controller);
    let third = tokio::spawn(async move {
        third_controller
            .acquire_job_slot(WorkClass::Maintenance, &RuntimeCancellation::default())
            .await
    });
    until(&controller, |s| s.queued_jobs == 1).await;
    let snapshot = controller.snapshot();
    assert_eq!(snapshot.active_jobs, 2);
    assert_eq!(snapshot.budget.job_slots, 2);
    assert!(!third.is_finished());
    drop(second);
    let third = tokio::time::timeout(DEADLINE, third)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(third.class(), WorkClass::Maintenance);
    drop((first, third, model));
    until(&controller, |s| {
        s.active_jobs == 0 && s.active_work_class.is_none()
    })
    .await;
}

#[tokio::test]
async fn low_resource_budget_admits_one_job_and_cancelled_waiters_leave_no_state() {
    let controller = Arc::new(AdmissionController::new(ResourceBudget::LOW_RESOURCE));
    let held = controller
        .acquire_job_slot(WorkClass::Explicit, &RuntimeCancellation::default())
        .await
        .unwrap();
    let cancel = RuntimeCancellation::default();
    let token = cancel.clone();
    let waiting_controller = Arc::clone(&controller);
    let waiting = tokio::spawn(async move {
        waiting_controller
            .acquire_job_slot(WorkClass::Explicit, &token)
            .await
    });
    until(&controller, |s| s.queued_jobs == 1).await;
    cancel.cancel();
    assert!(waiting.await.unwrap().is_err());
    until(&controller, |s| s.queued_jobs == 0 && s.active_jobs == 1).await;
    drop(held);
    until(&controller, |s| s.active_jobs == 0).await;
}

#[tokio::test]
async fn checkpoint_yields_only_to_higher_priority_or_playback() {
    let controller = Arc::new(AdmissionController::new(ResourceBudget::LOW_RESOURCE));
    let cancel = RuntimeCancellation::default();
    let mut slot = controller
        .acquire_job_slot(WorkClass::Metadata, &cancel)
        .await
        .unwrap();
    assert!(!slot.must_yield());

    // Equal or lower priority waiters do not preempt.
    let lower_controller = Arc::clone(&controller);
    let lower = tokio::spawn(async move {
        lower_controller
            .acquire_job_slot(WorkClass::Maintenance, &RuntimeCancellation::default())
            .await
    });
    until(&controller, |s| s.queued_jobs == 1).await;
    assert!(!slot.must_yield());
    slot.checkpoint(&cancel).await.unwrap();
    assert_eq!(controller.snapshot().active_jobs, 1);

    // A higher-priority waiter takes the slot at the next checkpoint.
    let (higher_started, higher_running) = tokio::sync::oneshot::channel();
    let (finish_higher, higher_finish) = tokio::sync::oneshot::channel::<()>();
    let higher_controller = Arc::clone(&controller);
    let higher = tokio::spawn(async move {
        let slot = higher_controller
            .acquire_job_slot(WorkClass::Explicit, &RuntimeCancellation::default())
            .await
            .unwrap();
        higher_started.send(()).unwrap();
        higher_finish.await.unwrap();
        drop(slot);
    });
    until(&controller, |s| s.queued_jobs == 2).await;
    assert!(slot.must_yield());
    let resumed = tokio::spawn(async move {
        slot.checkpoint(&RuntimeCancellation::default())
            .await
            .map(|_| slot)
    });
    tokio::time::timeout(DEADLINE, higher_running)
        .await
        .unwrap()
        .unwrap();
    assert!(!resumed.is_finished());
    finish_higher.send(()).unwrap();
    higher.await.unwrap();
    // Metadata outranks the earlier maintenance waiter when the slot frees.
    let mut slot = tokio::time::timeout(DEADLINE, resumed)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(!lower.is_finished());

    // Playback suspends metadata at its next checkpoint, then resumes it.
    let playback = controller.playback_started();
    assert!(slot.must_yield());
    let resumed = tokio::spawn(async move {
        slot.checkpoint(&RuntimeCancellation::default())
            .await
            .map(|_| slot)
    });
    // Maintenance is suspended too, so nothing runs during playback.
    until(&controller, |s| s.active_jobs == 0 && s.suspended_work == 2).await;
    drop(playback);
    let slot = tokio::time::timeout(DEADLINE, resumed)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    drop(slot);
    tokio::time::timeout(DEADLINE, lower)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    until(&controller, |s| s.active_jobs == 0 && s.queued_jobs == 0).await;
}

#[tokio::test]
async fn checkpoint_observes_cancellation_and_releases_the_slot() {
    let controller = Arc::new(AdmissionController::new(ResourceBudget::LOW_RESOURCE));
    let cancel = RuntimeCancellation::default();
    let mut slot = controller
        .acquire_job_slot(WorkClass::Background, &cancel)
        .await
        .unwrap();
    cancel.cancel();
    assert!(slot.checkpoint(&cancel).await.is_err());
    assert_eq!(controller.snapshot().active_jobs, 0);
    // A cancelled job suspended during playback also returns promptly.
    let cancel = RuntimeCancellation::default();
    let mut slot = controller
        .acquire_job_slot(WorkClass::Background, &cancel)
        .await
        .unwrap();
    let _playback = controller.playback_started();
    let token = cancel.clone();
    let waiting = tokio::spawn(async move { slot.checkpoint(&token).await });
    until(&controller, |s| s.suspended_work == 1).await;
    cancel.cancel();
    assert!(tokio::time::timeout(DEADLINE, waiting)
        .await
        .unwrap()
        .unwrap()
        .is_err());
    until(&controller, |s| s.suspended_work == 0 && s.active_jobs == 0).await;
}

#[tokio::test]
async fn panicking_holders_release_permits_and_slots() {
    let controller = Arc::new(AdmissionController::new(ResourceBudget::LOW_RESOURCE));
    let panicking = Arc::clone(&controller);
    let result = tokio::spawn(async move {
        let _slot = panicking
            .acquire_job_slot(WorkClass::Metadata, &RuntimeCancellation::default())
            .await
            .unwrap();
        let _permit = panicking
            .admit(WorkClass::Background, &RuntimeCancellation::default())
            .await
            .unwrap();
        panic!("synthetic failure while holding resources");
    })
    .await;
    assert!(result.is_err());
    let snapshot = controller.snapshot();
    assert_eq!(snapshot.active_jobs, 0);
    assert_eq!(snapshot.active_work_class, None);
    tokio::time::timeout(
        DEADLINE,
        controller.acquire_job_slot(WorkClass::Maintenance, &RuntimeCancellation::default()),
    )
    .await
    .expect("state not corrupted by the failed task")
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mixed_contention_with_playback_toggling_never_deadlocks() {
    let controller = Arc::new(AdmissionController::new(ResourceBudget::DEFAULT));
    let mut tasks = tokio::task::JoinSet::new();
    for i in 0..240usize {
        let controller = Arc::clone(&controller);
        let class = WorkClass::ALL[i % WorkClass::ALL.len()];
        tasks.spawn(async move {
            let cancel = RuntimeCancellation::default();
            if i % 3 == 0 {
                let mut slot = controller.acquire_job_slot(class, &cancel).await.unwrap();
                for _ in 0..3 {
                    tokio::task::yield_now().await;
                    slot.checkpoint(&cancel).await.unwrap();
                }
            } else if i % 11 == 0 {
                // Some waiters give up; they must leave no queued state.
                let cancel = RuntimeCancellation::default();
                cancel.cancel();
                assert!(controller.admit(class, &cancel).await.is_err());
            } else {
                let _permit = controller.admit(class, &cancel).await.unwrap();
                tokio::task::yield_now().await;
            }
        });
    }
    let toggler = {
        let controller = Arc::clone(&controller);
        tokio::spawn(async move {
            for _ in 0..50 {
                let guard = controller.playback_started();
                tokio::task::yield_now().await;
                drop(guard);
                tokio::task::yield_now().await;
            }
        })
    };
    tokio::time::timeout(Duration::from_secs(20), async {
        while let Some(result) = tasks.join_next().await {
            result.unwrap();
        }
    })
    .await
    .expect("governor deadlocked");
    toggler.await.unwrap();
    let snapshot = controller.snapshot();
    assert_eq!(snapshot.queued_work + snapshot.queued_jobs, 0);
    assert_eq!(snapshot.active_jobs, 0);
    assert_eq!(snapshot.active_work_class, None);
    assert_eq!(snapshot.playback_sessions, 0);
}
