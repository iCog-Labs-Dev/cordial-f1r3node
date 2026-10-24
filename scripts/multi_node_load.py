#!/usr/bin/env python3
"""Paced HTTP deploy load with finalization observed on every configured node.

Python 3.10+, standard library only. Reads fresh, already signed HTTP requests
from JSONL. Does not start nodes, alter chain configuration or force proposals.
"""
import argparse
import asyncio
from concurrent.futures import ThreadPoolExecutor
import datetime as dt
import hashlib
import json
import math
from pathlib import Path
import re
import statistics
import subprocess
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid
from benchmark import provenance

def request_json(url, timeout, payload=None):
    data = None if payload is None else json.dumps(payload).encode()
    request = urllib.request.Request(
        url, data=data, headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(request, timeout=timeout) as response:
        return json.loads(response.read())

def percentiles(values):
    if not values:
        return {"count": 0, "median": None, "p95": None, "p99": None}
    values = sorted(values)
    def nearest(p):
        return values[max(0, math.ceil(p * len(values)) - 1)]
    return {"count": len(values), "median": statistics.median(values),
            "p95": nearest(.95), "p99": nearest(.99)}

def memory_bytes(text):
    number, unit = re.fullmatch(r"\s*([0-9.]+)\s*([A-Za-z]+)\s*", text).groups()
    factors = {"B": 1, "kB": 1000, "KB": 1000, "MB": 1000**2, "GB": 1000**3,
               "KiB": 1024, "MiB": 1024**2, "GiB": 1024**3, "TiB": 1024**4}
    return int(float(number) * factors[unit])

def docker_stats(containers):
    command = ["docker", "stats", "--no-stream", "--format", "{{json .}}", *containers]
    result = subprocess.run(command, text=True, capture_output=True, timeout=15, check=True)
    records = []
    for line in result.stdout.splitlines():
        row = json.loads(line)
        records.append({
            "container": row["Name"],
            "cpu_percent": float(row["CPUPerc"].rstrip("%")),
            "memory_usage_bytes": memory_bytes(row["MemUsage"].split("/")[0]),
        })
    if len(records) != len(containers):
        raise RuntimeError("docker stats did not return every configured container")
    return records

async def run(args, requests, output, report):
    loop = asyncio.get_running_loop()
    loop.set_default_executor(ThreadPoolExecutor(max_workers=args.concurrency + 2))
    limiter = asyncio.Semaphore(args.concurrency)
    async def http(url, payload=None):
        async with limiter:
            return await asyncio.to_thread(request_json, url, args.request_timeout, payload)
    # Readiness and Docker sampling fail before applying any offered load.
    report["nodes_before"] = dict(zip(
        args.node, await asyncio.gather(*(http(n + "/api/status") for n in args.node))))
    for node, status in report["nodes_before"].items():
        if isinstance(status, dict) and status.get("isReady") is False:
            raise RuntimeError("node is not ready: " + node)
    report["initial_resources"] = await asyncio.to_thread(docker_stats, args.container)
    # Refuse existing deployments: replaying the input would invalidate latency.
    for deploy in requests:
        try:
            existing = await http(args.node[0] + "/api/deploy/" + deploy["signature"] + "?view=summary")
        except urllib.error.HTTPError as error:
            if error.code not in (400, 404):
                raise
        else:
            if isinstance(existing, dict) and existing.get("blockHash"):
                raise ValueError("input contains an already included deploy; generate fresh JSONL")
    origin = time.monotonic()
    offer_seconds = len(requests) / args.rate
    records = []
    resource_errors = []
    resource_rows = []
    report["measurement_started_at_utc"] = dt.datetime.now(dt.timezone.utc).isoformat()
    stop = asyncio.Event()
    resource_path = output / "resources.jsonl"
    backlog_path = output / "backlog.jsonl"
    raw_path = output / "deploys.jsonl"

    async def monitor():
        with resource_path.open("w") as resources, backlog_path.open("w") as backlog:
            while not stop.is_set():
                tick = time.monotonic() - origin
                try:
                    rows = await asyncio.to_thread(docker_stats, args.container)
                    resource_rows.extend(rows)
                    resources.write(json.dumps({"elapsed_seconds": tick, "nodes": rows}) + "\n")
                    resources.flush()
                except Exception as error:
                    resource_errors.append({"elapsed_seconds": tick, "error": str(error)})
                backlog.write(json.dumps({
                    "elapsed_seconds": time.monotonic() - origin,
                    "submitted": sum("submitted_seconds" in r for r in records),
                    "accepted": sum(r.get("accepted", False) for r in records),
                    "finalized_on_all_nodes": sum(r.get("outcome") == "finalized" for r in records),
                    "accepted_not_finalized": sum(r.get("accepted", False) and r.get("outcome") != "finalized" for r in records),
                }) + "\n")
                backlog.flush()
                try:
                    await asyncio.wait_for(stop.wait(), timeout=args.poll_interval)
                except asyncio.TimeoutError:
                    pass

    async def measure(index, deploy):
        scheduled = origin + index / args.rate
        await asyncio.sleep(max(0, scheduled - time.monotonic()))
        row = {"signature": deploy["signature"], "submission_node": args.node[index % len(args.node)],
               "scheduled_seconds": scheduled - origin, "finalized": {}, "accepted": False,
               "poll_errors": 0}
        records.append(row)
        try:
            # Timestamp inside the HTTP concurrency limit: queueing is reported
            # separately as scheduling lag and included in offered-load latency.
            async with limiter:
                started = time.monotonic()
                row["submitted_seconds"] = started - origin
                row["scheduling_lag_seconds"] = started - scheduled
                response = await asyncio.to_thread(
                    request_json, row["submission_node"] + "/api/deploy", args.request_timeout, deploy)
            if isinstance(response, dict) and (response.get("success") is False or "error" in response):
                raise RuntimeError("node rejected deploy: " + json.dumps(response))
            row["accepted"] = True
            row["accepted_seconds"] = time.monotonic() - origin
            deadline = started + args.settle_seconds
            while time.monotonic() < deadline:
                missing = [n for n in args.node if n not in row["finalized"]]
                results = await asyncio.gather(*(
                    http(n + "/api/deploy/" + deploy["signature"] + "?view=summary")
                    for n in missing), return_exceptions=True)
                for node, result in zip(missing, results):
                    if isinstance(result, Exception):
                        row["poll_errors"] += 1
                        row["last_poll_error"] = str(result)
                        continue
                    if (isinstance(result, dict)
                            and result.get("deployId", "").lower() == deploy["signature"]
                            and result.get("isFinalized") is True
                            and isinstance(result.get("errored"), bool)
                            and result.get("blockHash")):
                        row["finalized"][node] = {
                            "observed_seconds": time.monotonic() - origin,
                            "block_hash": result["blockHash"], "execution_errored": result["errored"],
                        }
                if len(row["finalized"]) == len(args.node):
                    hashes = {value["block_hash"] for value in row["finalized"].values()}
                    row["outcome"] = "finalized" if len(hashes) == 1 else "inconsistent_finalized_blocks"
                    row["execution_errored"] = any(value["execution_errored"] for value in row["finalized"].values())
                    observed = max(value["observed_seconds"] for value in row["finalized"].values())
                    row["latency_seconds"] = observed - row["submitted_seconds"]
                    row["offered_latency_seconds"] = observed - row["scheduled_seconds"]
                    return
                await asyncio.sleep(args.poll_interval)
            row["outcome"] = "finalization_timeout"
        except Exception as error:
            row["outcome"] = "submission_error" if not row["accepted"] else "observation_error"
            row["error"] = str(error)
        finally:
            with raw_path.open("a") as raw:
                raw.write(json.dumps(row) + "\n")

    monitor_task = asyncio.create_task(monitor())
    try:
        await asyncio.gather(*(measure(i, deploy) for i, deploy in enumerate(requests)))
    finally:
        elapsed = time.monotonic() - origin
        stop.set()
        await monitor_task
        report["deploys_recorded"] = len(records)
        report["resource_errors"] = resource_errors
        report["elapsed_seconds"] = elapsed
    finalized = [r for r in records if r.get("outcome") == "finalized"]
    successful = [r for r in finalized if not r["execution_errored"]]
    during = [r for r in finalized if max(v["observed_seconds"] for v in r["finalized"].values()) <= offer_seconds]
    report.update({
        "offered_deploys": len(requests),
        "offered_rate_per_second": args.rate,
        "offered_window_seconds": offer_seconds,
        "accepted": sum(r["accepted"] for r in records),
        "finalized_on_all_nodes": len(finalized),
        "successful_execution_finalized": len(successful),
        "finalized_execution_errors": len(finalized) - len(successful),
        "backlog_at_end": sum(r["accepted"] and r.get("outcome") != "finalized" for r in records),
        "observed_finalized_per_second_in_offered_window": len(during) / offer_seconds,
        "observed_finalized_per_second_including_drain": len(finalized) / max(elapsed, offer_seconds),
        "finalization_latency_seconds": percentiles([r["latency_seconds"] for r in finalized]),
        "successful_execution_latency_seconds": percentiles([r["latency_seconds"] for r in successful]),
        "offered_load_latency_seconds": percentiles([r["offered_latency_seconds"] for r in finalized]),
        "submission_scheduling_lag_seconds": percentiles([r["scheduling_lag_seconds"] for r in records if "scheduling_lag_seconds" in r]),
        "per_node": {node: {
            "finalized": sum(node in r["finalized"] for r in records),
            "latency_seconds": percentiles([
                r["finalized"][node]["observed_seconds"] - r["submitted_seconds"]
                for r in records if node in r["finalized"]]),
        } for node in args.node},
        "latency_note": "Client-observed upper bounds; include HTTP and polling delay. Timeouts are excluded from percentile distributions and reported in counts.",
        "resource_summary": {
            container: {
                "samples": len([r for r in resource_rows if r["container"] == container]),
                "mean_cpu_percent": statistics.mean(r["cpu_percent"] for r in resource_rows if r["container"] == container),
                "max_cpu_percent": max(r["cpu_percent"] for r in resource_rows if r["container"] == container),
                "max_memory_usage_bytes": max(r["memory_usage_bytes"] for r in resource_rows if r["container"] == container),
            } for container in sorted({r["container"] for r in resource_rows})
        },
        "resource_note": "Docker CPU percent and Docker-reported memory usage; memory is not process RSS. Container ordering corresponds to --node ordering.",
        "complete": len(successful) == len(requests) and not resource_errors and bool(resource_rows),
    })
    return 0 if report["complete"] else 2

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--node", action="append", required=True, help="base HTTP URL; repeat for every node")
    parser.add_argument("--container", action="append", required=True, help="Docker name/ID; one per node in the same order")
    parser.add_argument("--requests", type=Path, required=True, help="fresh signed HTTP deploy JSONL")
    parser.add_argument("--rate", type=float, default=1)
    parser.add_argument("--count", type=int, default=100)
    parser.add_argument("--concurrency", type=int, default=16)
    parser.add_argument("--settle-seconds", type=float, default=120)
    parser.add_argument("--poll-interval", type=float, default=1)
    parser.add_argument("--request-timeout", type=float, default=5)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    args.node = [n.rstrip("/") for n in args.node]
    if len(args.node) < 2 or len(set(args.node)) != len(args.node):
        parser.error("provide at least two distinct --node URLs")
    if len(args.container) != len(args.node) or len(set(args.container)) != len(args.container):
        parser.error("provide one distinct --container per --node, in the same order")
    for node in args.node:
        if urllib.parse.urlparse(node).scheme not in ("http", "https"):
            parser.error("--node must be an HTTP(S) URL")
    if args.count <= 0 or args.concurrency <= 0:
        parser.error("--count and --concurrency must be positive")
    for key in ["rate", "settle_seconds", "poll_interval", "request_timeout"]:
        value = getattr(args, key)
        if not math.isfinite(value) or value <= 0:
            parser.error(key.replace("_", "-") + " must be finite and positive")
    content = args.requests.read_bytes()
    corpus = [json.loads(line) for line in content.splitlines() if line.strip()]
    if len(corpus) < args.count:
        parser.error("JSONL has fewer requests than --count")
    corpus = corpus[:args.count]
    signatures = set()
    for request in corpus:
        sig = request.get("signature", "").removeprefix("0x").lower()
        try:
            if not sig or not bytes.fromhex(sig):
                raise ValueError()
        except ValueError:
            parser.error("every request requires a nonempty hex signature")
        if sig in signatures:
            parser.error("duplicate signature in input")
        if not all(k in request for k in ("data", "deployer", "sigAlgorithm")):
            parser.error("request must have data, deployer, signature and sigAlgorithm")
        signatures.add(sig)
        request["signature"] = sig
    stamp = dt.datetime.now(dt.timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    output = args.output or Path("target/load-runs") / (stamp + "-" + uuid.uuid4().hex[:8])
    output.mkdir(parents=True, exist_ok=False)
    (output / "requests.jsonl").write_bytes(content)
    report = {"metadata": provenance(), "configuration": {
        k: str(v) if isinstance(v, Path) else v for k, v in vars(args).items()
    }, "input_sha256": hashlib.sha256(content).hexdigest()}
    code = 1
    try:
        code = asyncio.run(run(args, corpus, output, report))
    except (Exception, KeyboardInterrupt) as error:
        report["fatal_error"] = str(error) or type(error).__name__
        report["complete"] = False
    finally:
        (output / "summary.json").write_text(json.dumps(report, indent=2) + "\n")
        print(json.dumps({"results": str(output), "complete": report.get("complete", False),
                          "exit_code": code}, indent=2))
    return code

if __name__ == "__main__":
    sys.exit(main())
