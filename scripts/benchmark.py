#!/usr/bin/env python3
"""Run selected Criterion targets and archive provenance, logs and reports."""
import argparse
import datetime as dt
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import signal
import subprocess
import sys
import time
import uuid

ROOT = Path(__file__).resolve().parent.parent
HOST_DEPENDENCY = ROOT.parent / "f1r3node-rust"
PACKAGES = [
    "cordial-miners-core", "cordial-f1r3node-adapter",
    "cordial-por", "cordial-app-runtime",
]

def capture(args, cwd=ROOT):
    try:
        result = subprocess.run(args, cwd=cwd, text=True, stdout=subprocess.PIPE,
                                stderr=subprocess.STDOUT, check=False)
    except OSError as error:
        # Preserve a useful run manifest even when a tool/checkout is unavailable.
        return {"exit_code": 127, "output": f"{type(error).__name__}: {error}"}
    return {"exit_code": result.returncode, "output": result.stdout.strip()}

def provenance():
    diff = capture(["git", "diff", "--binary", "HEAD"])
    cpu = Path("/proc/cpuinfo")
    memory = Path("/proc/meminfo")
    return {
        "recorded_at_utc": dt.datetime.now(dt.timezone.utc).isoformat(),
        "commit": capture(["git", "rev-parse", "HEAD"]),
        "working_tree": capture(["git", "status", "--porcelain"]),
        "tracked_diff_sha256": hashlib.sha256(diff["output"].encode()).hexdigest(),
        "untracked_source_sha256": {
            name: hashlib.sha256((ROOT / name).read_bytes()).hexdigest()
            for name in capture(["git", "ls-files", "--others", "--exclude-standard"])["output"].splitlines()
            if (ROOT / name).is_file()
        },
        "host_dependency_path": str(HOST_DEPENDENCY),
        "host_dependency_commit": capture(["git", "rev-parse", "HEAD"], HOST_DEPENDENCY),
        "host_dependency_working_tree": capture(["git", "status", "--porcelain"], HOST_DEPENDENCY),
        "rustc": capture(["rustc", "-Vv"]),
        "cargo": capture(["cargo", "-V"]),
        "platform": platform.platform(),
        "machine": platform.machine(),
        "logical_cpus": os.cpu_count(),
        "cpuinfo": cpu.read_text() if cpu.exists() else platform.processor(),
        "memory": memory.read_text() if memory.exists() else None,
        "environment": {key: os.environ.get(key) for key in [
            "CORDIAL_BENCH_LARGE", "CORDIAL_BENCH_RSPACE", "CORDIAL_BENCH_SMOKE",
            "RUSTUP_TOOLCHAIN",
            "RUSTFLAGS", "CARGO_BUILD_TARGET", "CARGO_TARGET_DIR",
        ]},
    }

def utc_now():
    return dt.datetime.now(dt.timezone.utc).isoformat()


def save_metadata(manifest, metadata):
    # Readers should see either the previous or the complete updated document.
    temporary = manifest.with_suffix(".json.tmp")
    temporary.write_text(json.dumps(metadata, indent=2) + "\n")
    temporary.replace(manifest)


def criterion_options(extra, smoke=False):
    extra = list(extra)
    if extra[:1] == ["--"]:
        extra = extra[1:]
    smoke = smoke or "--test" in extra
    if smoke and "--test" not in extra:
        extra.append("--test")
    return extra, smoke


def benchmark_targets(packages, selected=None):
    # Select explicit bench targets; --benches also runs library/binary harnesses.
    result = capture(["cargo", "metadata", "--offline", "--no-deps", "--format-version", "1"])
    if result["exit_code"]:
        raise RuntimeError("Cannot discover benchmark targets: " + result["output"])
    workspace = json.loads(result["output"])
    members = {
        package["name"]: package for package in workspace["packages"]
        if package["id"] in workspace["workspace_members"]
    }
    targets = []
    for package in packages:
        names = sorted(
            target["name"] for target in members[package]["targets"]
            if "bench" in target["kind"]
        )
        if selected:
            if selected not in names:
                raise ValueError(f"Unknown benchmark {package}/{selected}; choose from {names}")
            names = [selected]
        targets.extend((package, name) for name in names)
    return targets


def stop_process_tree(process):
    # Cargo launches benchmark executables. Stopping Cargo alone can orphan them.
    if os.name == "posix":
        try:
            os.killpg(process.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            process.wait()
    elif os.name == "nt":
        subprocess.run(["taskkill", "/PID", str(process.pid), "/T", "/F"],
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False)
        process.wait()
    else:
        process.terminate()
        process.wait()


def run_target(package, bench, extra, output, manifest, metadata):
    command = ["cargo", "bench", "-p", package, "--bench", bench]
    if extra:
        command += ["--", *extra]
    print("+", " ".join(command), flush=True)
    log_path = output / f"{package}-{bench}.log"
    record = {
        "package": package, "bench": bench, "command": command,
        "started_at_utc": utc_now(), "status": "running",
        "exit_code": None, "log": log_path.name,
    }
    metadata["runs"].append(record)
    save_metadata(manifest, metadata)
    started = time.monotonic()
    process = None
    try:
        with log_path.open("w", buffering=1) as log:
            process = subprocess.Popen(
                command, cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                text=True, bufsize=1, start_new_session=(os.name == "posix"),
            )
            for line in process.stdout:
                print(line, end="", flush=True)
                log.write(line)
                log.flush()
            code = process.wait()
        record.update(exit_code=code, status="passed" if code == 0 else "failed")
        return code
    except BaseException as error:
        if process is not None:
            stop_process_tree(process)
        record.update(
            exit_code=process.returncode if process is not None else None,
            status="interrupted" if isinstance(error, KeyboardInterrupt) else "failed",
            error=f"{type(error).__name__}: {error}",
        )
        raise
    finally:
        if process is not None and process.stdout is not None:
            process.stdout.close()
        record["finished_at_utc"] = utc_now()
        record["elapsed_seconds"] = time.monotonic() - started
        save_metadata(manifest, metadata)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--package", action="append", choices=PACKAGES)
    parser.add_argument("--bench", help="one target; requires exactly one --package")
    parser.add_argument("--smoke", action="store_true",
                        help="small fixtures and one invocation per case; no timing reports")
    parser.add_argument("--large", action="store_true")
    parser.add_argument("--rspace", action="store_true")
    parser.add_argument("--output", type=Path)
    parser.add_argument("criterion_args", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    packages = args.package or PACKAGES
    if args.bench and len(packages) != 1:
        parser.error("--bench requires exactly one --package")
    extra, smoke = criterion_options(args.criterion_args, args.smoke)
    if smoke and args.large:
        parser.error("--large cannot be combined with smoke mode (--smoke or -- --test)")
    if os.environ.get("CRITERION_HOME"):
        parser.error("unset CRITERION_HOME so reports can be archived consistently")
    # Set explicit modes; do not inherit expensive workloads accidentally.
    os.environ["CORDIAL_BENCH_LARGE"] = "1" if args.large else "0"
    os.environ["CORDIAL_BENCH_RSPACE"] = "1" if args.rspace else "0"
    os.environ["CORDIAL_BENCH_SMOKE"] = "1" if smoke else "0"
    if smoke:
        print("Smoke mode: reduced fixtures; no warmup, sampling or Criterion reports.",
              flush=True)
    stamp = dt.datetime.now(dt.timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    output = args.output or ROOT / "target" / "benchmark-runs" / (stamp + "-" + uuid.uuid4().hex[:8])
    output.mkdir(parents=True, exist_ok=False)
    metadata = provenance()
    metadata["configuration"] = {
        "packages": packages, "bench": args.bench, "large": args.large,
        "rspace": args.rspace, "smoke": smoke, "criterion_args": extra,
    }
    metadata["status"] = "running"
    metadata["runs"] = []
    manifest = output / "metadata.json"
    save_metadata(manifest, metadata)
    status = 0
    try:
        targets = benchmark_targets(packages, args.bench)
        metadata["planned_targets"] = [
            {"package": package, "bench": bench} for package, bench in targets
        ]
        save_metadata(manifest, metadata)
        for index, (package, bench) in enumerate(targets, 1):
            print(f"[{index}/{len(targets)}] {package}/{bench}", flush=True)
            code = run_target(package, bench, extra, output, manifest, metadata)
            if code:
                status = code if code > 0 else 128 - code
                break
        metadata["status"] = "passed" if status == 0 else "failed"
    except KeyboardInterrupt:
        status = 130
        metadata["status"] = "interrupted"
        print("\nInterrupted; stopped the Cargo/benchmark process group.", flush=True)
    except BaseException as error:
        metadata.update(status="failed", error=f"{type(error).__name__}: {error}")
        raise
    finally:
        try:
            if smoke:
                # --test produces no statistics. Copying old reports is misleading
                # and can make a smoke check slow when the report tree is large.
                metadata["report_note"] = (
                    "Smoke check only; no Criterion reports generated or archived."
                )
            else:
                target = Path(os.environ.get("CARGO_TARGET_DIR", str(ROOT / "target")))
                if not target.is_absolute():
                    target = ROOT / target
                reports = target / "criterion"
                if reports.exists():
                    shutil.copytree(reports, output / "criterion")
                metadata["report_note"] = (
                    "Criterion tree includes retained baselines and may contain older cases. "
                    "Use this run's logs/configuration to identify executed cases."
                )
        finally:
            metadata["finished_at_utc"] = utc_now()
            save_metadata(manifest, metadata)
            print("Results:", output, flush=True)
    return status


if __name__ == "__main__":
    sys.exit(main())
