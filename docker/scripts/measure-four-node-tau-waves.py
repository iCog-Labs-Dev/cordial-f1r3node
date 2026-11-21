#!/usr/bin/env python3
"""Time live four-node Cordial tau convergence with genesis bonds and no PoR."""

import argparse
import hashlib
import json
import os
import queue
import statistics
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.request
from pathlib import Path


NODES = (
    ("validator-1", 51401),
    ("validator-2", 52401),
    ("validator-3", 53401),
    ("validator-4", 54401),
)
EVENT_PREFIX = "TAU_WAVE_EVENT "


def read_output(name, process, log_path, events):
    with log_path.open("w", encoding="utf-8") as log:
        for line in process.stdout:
            log.write(line)
            log.flush()
            if line.startswith(EVENT_PREFIX):
                try:
                    events.put((name, json.loads(line[len(EVENT_PREFIX) :])))
                except json.JSONDecodeError as error:
                    events.put((name, {"error": f"invalid wave event: {error}"}))


def fetch_ordered_hashes(port):
    url = f"http://127.0.0.1:{port}/ordered-output/latest"
    with urllib.request.urlopen(url, timeout=5) as response:
        output = json.load(response)
    return [bytes(block["content_hash"]).hex() for block in output["blocks"]]


def common_prefix_length(sequences):
    if not sequences:
        return 0
    count = min(map(len, sequences))
    first = sequences[0]
    for other in sequences[1:]:
        for index in range(count):
            if first[index] != other[index]:
                return index
    return count


def sha256_file(path):
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def check_full_history(grpc_port, depth):
    url = f"http://127.0.0.1:{grpc_port + 2}/api/blocks/{depth}"
    with urllib.request.urlopen(url, timeout=10) as response:
        blocks = json.load(response)
    heights = [int(block.get("blockInfo", block)["blockNumber"]) for block in blocks]
    if not heights or min(heights) != 0:
        raise RuntimeError(
            f"node {grpc_port} does not expose genesis in its latest {depth} blocks; "
            "run on a fresh cluster or increase --depth"
        )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=Path("target/debug/ordered_output_server"))
    parser.add_argument("--bonds-file", type=Path, default=Path("docker/genesis/cordial-bonds.txt"))
    parser.add_argument("--evidence-dir", type=Path, required=True)
    parser.add_argument("--waves", type=int, default=3)
    parser.add_argument("--timeout-seconds", type=int, default=600)
    parser.add_argument("--poll-ms", type=int, default=1000)
    parser.add_argument("--depth", type=int, default=256)
    args = parser.parse_args()
    if args.waves < 1 or args.timeout_seconds < 1 or args.poll_ms < 1 or args.depth < 1:
        parser.error("waves, timeout-seconds, poll-ms, and depth must be positive")
    if not args.binary.is_file() or not os.access(args.binary, os.X_OK):
        parser.error(f"binary is missing or not executable: {args.binary}")
    if not args.bonds_file.is_file():
        parser.error(f"bonds file is missing: {args.bonds_file}")
    args.evidence_dir.mkdir(parents=True, exist_ok=False)

    events = queue.Queue()
    processes = {}
    observed = {}
    finalized = {}
    samples = []
    wave_status = {}
    deadline = time.monotonic() + args.timeout_seconds
    inputs = {
        "binary": str(args.binary),
        "binary_sha256": sha256_file(args.binary),
        "bonds_file": str(args.bonds_file),
        "bonds_sha256": sha256_file(args.bonds_file),
        "probe_source_sha256": sha256_file(Path(__file__)),
        "server_source_sha256": sha256_file(Path("crates/cordial-f1r3node-adapter/src/bin/ordered_output_server.rs")),
        "git_commit": subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip(),
        "nodes": dict(NODES),
        "waves_requested": args.waves,
        "poll_ms": args.poll_ms,
        "depth": args.depth,
        "started_at_ns": time.time_ns(),
        "definition": "first observed Cordial wave in any mirror to first poll where four mirrors share its finalized tau prefix",
    }
    (args.evidence_dir / "inputs.json").write_text(json.dumps(inputs, indent=2) + "\n")

    result = {"outcome": "FAIL", "error": "interrupted", "samples": samples}
    try:
        for name, grpc_port in NODES:
            check_full_history(grpc_port, args.depth)
            print(f"{name}: genesis visible in native block history", flush=True)
        for index, (name, grpc_port) in enumerate(NODES):
            http_port = 7081 + index
            command = [
                str(args.binary), "--grpc-url", f"http://127.0.0.1:{grpc_port}",
                "--addr", f"127.0.0.1:{http_port}", "--bonds-file", str(args.bonds_file),
                "--depth", str(args.depth), "--poll-interval-ms", str(args.poll_ms),
                "--wave-events",
            ]
            process = subprocess.Popen(
                command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                text=True, bufsize=1, env={**os.environ, "RUST_LOG": "warn"},
            )
            processes[name] = process
            threading.Thread(
                target=read_output,
                args=(name, process, args.evidence_dir / f"{name}.log", events),
                daemon=True,
            ).start()
            print(f"{name}: started genesis-weight τ mirror on gRPC {grpc_port}", flush=True)

        while len(samples) < args.waves and time.monotonic() < deadline:
            for name, process in processes.items():
                if process.poll() is not None:
                    raise RuntimeError(f"{name} mirror exited ({process.returncode}); see {name}.log")
            try:
                name, event = events.get(timeout=args.poll_ms / 1000)
            except queue.Empty:
                event = None
            if event:
                if "error" in event:
                    raise RuntimeError(f"{name}: {event['error']}")
                wave = event["wave"]
                kind = event["kind"]
                if kind == "observed":
                    observed.setdefault(wave, {})[name] = event
                elif kind == "finalized":
                    finalized.setdefault(wave, {})[name] = event
                print(f"{name}: {kind} Cordial wave {wave} (round {event['round']})", flush=True)

            # A finalized anchor can jump over a wave. Only measure waves that
            # all four mirrors observed and finalized explicitly after startup.
            for wave in sorted(set(observed) & set(finalized)):
                if wave in wave_status:
                    continue
                if len(observed[wave]) != 4 or len(finalized[wave]) != 4:
                    continue
                if any(event["initial"] for event in observed[wave].values()):
                    wave_status[wave] = "startup"
                    continue
                if any(event["initial"] for event in finalized[wave].values()):
                    wave_status[wave] = "startup"
                    continue
                final_outputs = {
                    (event["anchor_hash"], event["tau_sha256"], event["ordered_len"])
                    for event in finalized[wave].values()
                }
                if len(final_outputs) != 1:
                    raise RuntimeError(f"wave {wave} finalized with different τ anchors or ordered hashes")
                required_len = min(event["ordered_len"] for event in finalized[wave].values())
                if required_len == 0:
                    raise RuntimeError(f"wave {wave} has an empty finalized τ output")
                try:
                    sequences = [fetch_ordered_hashes(7081 + index) for index in range(4)]
                except (urllib.error.URLError, ValueError, KeyError):
                    continue
                if common_prefix_length(sequences) < required_len:
                    continue
                agreed_at_ns = time.time_ns()
                first_observed_ns = min(int(event["at_ns"]) for event in observed[wave].values())
                last_finalized_ns = max(int(event["at_ns"]) for event in finalized[wave].values())
                sample = {
                    "wave": wave,
                    "round_range": [wave * 3, wave * 3 + 2],
                    "observed_at_ns": {n: e["at_ns"] for n, e in observed[wave].items()},
                    "finalized_at_ns": {n: e["at_ns"] for n, e in finalized[wave].items()},
                    "ordered_prefix_len": required_len,
                    "anchor_hash": finalized[wave]["validator-1"]["anchor_hash"],
                    "tau_sha256": finalized[wave]["validator-1"]["tau_sha256"],
                    "agreed_at_ns": str(agreed_at_ns),
                    "first_observation_to_agreement_seconds": round((agreed_at_ns - first_observed_ns) / 1e9, 3),
                    "first_observation_to_last_finalization_seconds": round((last_finalized_ns - first_observed_ns) / 1e9, 3),
                    "finalization_spread_seconds": round(
                        (last_finalized_ns - min(int(e["at_ns"]) for e in finalized[wave].values())) / 1e9, 3
                    ),
                }
                samples.append(sample)
                wave_status[wave] = "measured"
                print(
                    f"AGREED wave {wave}: {required_len} τ hashes, "
                    f"{sample['first_observation_to_agreement_seconds']:.3f}s "
                    "from first wave observation",
                    flush=True,
                )
                if len(samples) >= args.waves:
                    break

        if len(samples) < args.waves:
            raise TimeoutError(f"measured {len(samples)}/{args.waves} waves in {args.timeout_seconds}s")
        durations = [sample["first_observation_to_agreement_seconds"] for sample in samples]
        result = {
            "outcome": "PASS",
            "samples": samples,
            "median_seconds": statistics.median(durations),
            "min_seconds": min(durations),
            "max_seconds": max(durations),
        }
        print(f"PASS: {len(samples)} PoR-free τ waves; median={result['median_seconds']:.3f}s", flush=True)
        return 0
    except (RuntimeError, TimeoutError, urllib.error.URLError, ValueError, KeyError) as error:
        result = {"outcome": "FAIL", "error": str(error), "samples": samples}
        print(f"FAIL: {error}", file=sys.stderr, flush=True)
        return 1
    finally:
        for process in processes.values():
            process.terminate()
        for process in processes.values():
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()
        (args.evidence_dir / "result.json").write_text(json.dumps(result, indent=2) + "\n")


if __name__ == "__main__":
    raise SystemExit(main())
