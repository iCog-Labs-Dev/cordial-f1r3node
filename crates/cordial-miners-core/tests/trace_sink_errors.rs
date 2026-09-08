//! Each sink mode runs in its own process: no concurrent environment mutation.
#![cfg(feature = "trace")]

use cordial_miners_core::trace::{self, SchedulerTickEvent, TraceEvent};
use std::process::{Command, Output};

fn run_child(sink: &std::path::Path, mode: &str) -> Output {
    Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "trace_sink_child", "--nocapture"])
        .env("CORDIAL_TRACE_SINK_TEST", mode)
        .env("CORDIAL_TRACE_FILE", sink)
        .env(
            "CORDIAL_TRACE_STRICT",
            if mode == "strict" { "1" } else { "0" },
        )
        .output()
        .expect("run isolated trace sink test")
}

#[test]
fn trace_sink_child() {
    let Ok(mode) = std::env::var("CORDIAL_TRACE_SINK_TEST") else {
        return;
    };
    let event = TraceEvent::SchedulerTick(SchedulerTickEvent {
        node_id: "node-1".into(),
        tick: 1,
        wave: None,
    });
    if mode == "result" {
        assert!(trace::try_emit(&event).is_err());
    } else {
        trace::emit(event);
    }
}

fn assert_sink_failure(sink: &std::path::Path) {
    assert!(run_child(sink, "result").status.success());
    assert!(run_child(sink, "best-effort").status.success());
    let strict = run_child(sink, "strict");
    assert!(
        !strict.status.success(),
        "strict tracing ignored sink failure"
    );
    assert!(String::from_utf8_lossy(&strict.stderr).contains("TRACE EMISSION ERROR"));
}

#[test]
fn trace_sink_open_failure_is_fatal_only_in_strict_mode() {
    // Opening an existing directory as a file fails even when tests run as root.
    assert_sink_failure(&std::env::current_dir().unwrap());
}

#[cfg(target_os = "linux")]
#[test]
fn trace_sink_write_failure_is_fatal_only_in_strict_mode() {
    // Unlike the directory case, /dev/full opens successfully and fails on write.
    assert_sink_failure(std::path::Path::new("/dev/full"));
}
