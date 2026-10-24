//! Production event routing, duplicate protection, receipts and fresh replay.
//! The benchmark-local app only counts bytes; this measures runtime overhead,
//! not a marketplace, ledger or Rholang execution.
use cordial_app_runtime::*;
use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use std::time::Duration;

struct ByteCounter {
    id: AppId,
    bytes: u64,
}
impl CordialApp for ByteCounter {
    fn app_id(&self) -> &AppId {
        &self.id
    }
    fn validate(&self, event: &AppEvent) -> Result<(), AppError> {
        if event.event_type == "reject" {
            Err(AppError::ValidationRejected {
                message: "fixture rejection".into(),
            })
        } else {
            Ok(())
        }
    }
    fn apply(&mut self, event: AppEvent) -> Result<AppReceipt, AppError> {
        self.bytes += event.payload.len() as u64;
        Ok(AppReceipt {
            app_id: self.id.clone(),
            event_id: event.event_id,
            ordered_index: event.ordered_index,
            status: AppReceiptStatus::Applied,
            state_root: Some(self.bytes.to_le_bytes().to_vec()),
            message: None,
        })
    }
    fn snapshot(&self) -> AppSnapshot {
        AppSnapshot {
            app_id: self.id.clone(),
            cursor: None,
            state_root: Some(self.bytes.to_le_bytes().to_vec()),
            payload: self.bytes.to_le_bytes().to_vec(),
        }
    }
}
fn runtime() -> AppRuntime {
    let mut runtime = AppRuntime::new();
    for i in 0..4 {
        runtime.register_app(Box::new(ByteCounter {
            id: AppId(format!("app-{i}")),
            bytes: 0,
        }));
    }
    runtime
}
fn events(n: usize) -> Vec<AppEvent> {
    (0..n)
        .map(|i| AppEvent {
            event_id: AppEventId(format!("event-{i}")),
            app_id: AppId(format!("app-{}", i % 4)),
            event_type: if i % 5 == 0 {
                "reject".into()
            } else {
                "apply".into()
            },
            payload: vec![0xAB; 256],
            submitter: vec![1; 33],
            ordered_index: i as u64,
            block_hash: vec![2; 32],
            deploy_signature: None,
            finalized_anchor: vec![3; 32],
        })
        .collect()
}
fn bench_events(c: &mut Criterion) {
    let smoke = std::env::var("CORDIAL_BENCH_SMOKE").as_deref() == Ok("1")
        || std::env::args().any(|arg| arg == "--test");
    let sizes: &[usize] = if smoke {
        &[10]
    } else if std::env::var("CORDIAL_BENCH_LARGE").as_deref() == Ok("1") {
        &[100, 1_000, 10_000]
    } else {
        &[100, 1_000]
    };
    let mut group = c.benchmark_group("app_runtime");
    group.sample_size(20);
    group.measurement_time(Duration::from_secs(2));
    for &n in sizes {
        let values = events(n);
        let mut checked = runtime();
        let receipts: Vec<_> = checked
            .process_events(values.clone())
            .into_iter()
            .collect::<Result<_, _>>()
            .expect("application decisions");
        assert_eq!(receipts.len(), n);
        assert_eq!(
            receipts
                .iter()
                .filter(|r| r.status == AppReceiptStatus::Rejected)
                .count(),
            n.div_ceil(5)
        );
        assert_eq!(
            checked.cursor().expect("cursor").ordered_index,
            (n - 1) as u64
        );
        let mut replay = runtime();
        assert_eq!(
            replay
                .process_events(values.clone())
                .into_iter()
                .collect::<Result<Vec<_>, _>>()
                .expect("replay"),
            receipts
        );
        assert_eq!(replay.snapshots(), checked.snapshots());
        assert!(matches!(
            checked.process_event(values[0].clone()),
            Err(AppError::DuplicateEvent { .. })
        ));
        group.throughput(Throughput::Elements(n as u64));
        group.bench_function(BenchmarkId::new("fresh_replay_mixed_decisions", n), |b| {
            b.iter_batched_ref(
                || (runtime(), values.clone()),
                |(runtime, values)| runtime.process_events(values.drain(..)),
                BatchSize::PerIteration,
            );
        });
        group.bench_function(BenchmarkId::new("duplicate_rejection_batch", n), |b| {
            b.iter_batched_ref(
                || values.clone(),
                |values| checked.process_events(values.drain(..)),
                BatchSize::PerIteration,
            );
        });
    }
    group.finish();
}
criterion_group!(benches, bench_events);
criterion_main!(benches);
