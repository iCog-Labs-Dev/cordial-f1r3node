//! Opt-in, append-only observation of the deterministic convergence fixture.
//! No production protocol decisions depend on this diagnostic stream.
use serde_json::{Value, json};
use std::{
    cell::RefCell,
    fs::File,
    io::Write,
    time::{Instant, SystemTime, UNIX_EPOCH},
};
struct Writer {
    file: File,
    seq: u64,
    started: Instant,
    run_id: String,
    scenario: String,
}
thread_local! { static TRACE: RefCell<Option<Writer>> = const { RefCell::new(None) }; }
pub struct TraceGuard;
impl Drop for TraceGuard {
    fn drop(&mut self) {
        TRACE.with(|slot| {
            slot.borrow_mut().take();
        });
    }
}
pub fn start(scenario: &str, initial: [u64; 4], rounds: usize) -> TraceGuard {
    if let Ok(directory) = std::env::var("POR_EVENT_OUTPUT") {
        std::fs::create_dir_all(&directory).expect("create PoR event output directory");
        let run_id = format!(
            "{}-{}-{}",
            scenario,
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let file =
            File::create(std::path::Path::new(&directory).join(format!("{scenario}.events.jsonl")))
                .expect("create PoR event stream");
        TRACE.with(|slot| {
            *slot.borrow_mut() = Some(Writer {
                file,
                seq: 0,
                started: Instant::now(),
                run_id,
                scenario: scenario.into(),
            })
        });
        emit(
            "run_started",
            0,
            json!({"initial_weights": initial, "planned_rounds": rounds, "observer_count": 4, "fixture": "single-producer-chain", "blocks_per_window": 16}),
        );
    }
    TraceGuard
}
pub fn enabled() -> bool {
    TRACE.with(|slot| slot.borrow().is_some())
}
pub fn emit(kind: &str, round: u64, payload: Value) {
    TRACE.with(|slot| {
        let mut slot = slot.borrow_mut();
        if let Some(trace) = slot.as_mut() {
            let record = json!({"schema_version": 1, "kind": "por_event", "run_id": trace.run_id, "scenario": trace.scenario, "seq": trace.seq, "elapsed_ms": trace.started.elapsed().as_millis(), "event": kind, "round": round, "data": payload});
            let mut bytes = serde_json::to_vec(&record).expect("encode PoR event"); bytes.push(b'\n');
            trace.file.write_all(&bytes).expect("append PoR event");
            trace.seq += 1;
        }
    });
}
