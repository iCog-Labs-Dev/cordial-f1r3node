#!/usr/bin/env python3
"""Run four persistent PoR observers and check convergence across restarts."""

import argparse
import json
from pathlib import Path
import subprocess
import sys
import time


REPO = Path(__file__).resolve().parents[2]
NODES = ("cordial-validator-1", "cordial-validator-2", "cordial-validator-3", "cordial-validator-4")
PORTS = (51401, 52401, 53401, 54401)


def status_path(root, node):
    return root / node / "por" / "shadow-status.json"


def read_status(path, newer_than):
    try:
        if path.stat().st_mtime_ns <= newer_than:
            return None
        status = json.loads(path.read_text())
    except (FileNotFoundError, json.JSONDecodeError):
        return None
    if status.get("schema_version") != 1 or status.get("shadow_mode") is not True:
        raise ValueError(f"invalid PoR shadow status: {path}")
    return status


def compare_statuses(statuses, min_finalized):
    baseline = statuses[NODES[0]]
    minimum = min(len(statuses[node]["finalized_hashes"]) for node in NODES)
    if minimum < min_finalized or min(statuses[node]["source_height"] for node in NODES) < 1:
        raise ValueError(f"waiting for post-genesis finality ({minimum}/{min_finalized} hashes)")
    prefix = baseline["finalized_hashes"][:minimum]
    for node in NODES[1:]:
        status = statuses[node]
        if (status["active_por_round"], status["weight_commitment"], status["weights"]) != (
            baseline["active_por_round"], baseline["weight_commitment"], baseline["weights"]
        ):
            raise ValueError(f"{node} has a different PoR round, commitment, or weights")
        if status["finalized_hashes"][:minimum] != prefix:
            raise ValueError(f"{node} has a different finalized prefix")
        if len(status["finalized_hashes"]) == len(baseline["finalized_hashes"]):
            if status["finalized_anchor"] != baseline["finalized_anchor"]:
                raise ValueError(f"{node} has a different finalized anchor")
    return minimum


def check_recovery(before, after):
    if after["active_por_round"] < before["active_por_round"]:
        raise ValueError("PoR round regressed after restart")
    if after["finalized_hashes"][:len(before["finalized_hashes"])] != before["finalized_hashes"]:
        raise ValueError("finalized prefix changed or shrank after restart")
    if after["active_por_round"] == before["active_por_round"]:
        if (after["weight_commitment"], after["weights"]) != (
            before["weight_commitment"], before["weights"]
        ):
            raise ValueError("PoR projection changed at the same round after restart")


def wait_for_statuses(root, markers, runners, timeout, min_finalized):
    deadline = time.monotonic() + timeout
    last_error = "waiting for status files"
    while time.monotonic() < deadline:
        for node, process in runners.items():
            if process.poll() is not None:
                raise RuntimeError(f"{node} shadow exited with code {process.returncode}; see {root / node / 'runner.log'}")
        statuses = {node: read_status(status_path(root, node), markers[node]) for node in NODES}
        if all(status is not None for status in statuses.values()):
            try:
                count = compare_statuses(statuses, min_finalized)
                return statuses, count
            except (KeyError, TypeError, ValueError) as error:
                last_error = str(error)
        time.sleep(2)
    raise TimeoutError(f"four PoR shadows did not converge in {timeout}s: {last_error}")


def stop_runner(process):
    if process.poll() is None:
        process.terminate()
        try:
            process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--data-root", type=Path, required=True, help="persistent directory outside the repository")
    parser.add_argument("--binary", type=Path, default=REPO / "target/debug/live_por_shadow")
    parser.add_argument("--bonds-file", type=Path, default=REPO / "docker/genesis/cordial-bonds.txt")
    parser.add_argument("--timeout-seconds", type=int, default=300)
    parser.add_argument("--min-finalized-blocks", type=int, default=2)
    parser.add_argument("--skip-node-restart", action="store_true", help="only check runner recovery")
    args = parser.parse_args()
    if args.timeout_seconds <= 0 or args.min_finalized_blocks <= 0:
        parser.error("timeouts and minimum finalized blocks must be positive")
    binary = args.binary.resolve()
    bonds_file = args.bonds_file.resolve()
    root = args.data_root.resolve()
    if not binary.is_file() or not bonds_file.is_file():
        parser.error("build live_por_shadow and provide an existing bonds file first")
    if root == REPO or REPO in root.parents:
        parser.error("--data-root must be outside the repository")
    root.mkdir(parents=True, exist_ok=True)

    runners = {}
    logs = []

    def launch(node, port):
        directory = root / node
        directory.mkdir(parents=True, exist_ok=True)
        log = (directory / "runner.log").open("ab")
        logs.append(log)
        runners[node] = subprocess.Popen(
            [str(binary), "--grpc-url", f"http://127.0.0.1:{port}",
             "--bonds-file", str(bonds_file), "--data-dir", str(directory)],
            stdout=log, stderr=subprocess.STDOUT,
        )

    try:
        markers = {node: status_path(root, node).stat().st_mtime_ns
                   if status_path(root, node).exists() else 0 for node in NODES}
        for node, port in zip(NODES, PORTS):
            launch(node, port)
        statuses, count = wait_for_statuses(root, markers, runners, args.timeout_seconds,
                                            args.min_finalized_blocks)
        print(f"PASS: four shadows agree on round {statuses[NODES[0]]['active_por_round']}, "
              f"weight commitment {statuses[NODES[0]]['weight_commitment']}, "
              f"and {count} finalized hashes")

        node = NODES[0]
        before = statuses[node]
        stop_runner(runners[node])
        markers = {name: 0 for name in NODES}
        markers[node] = status_path(root, node).stat().st_mtime_ns
        launch(node, PORTS[0])
        statuses, count = wait_for_statuses(root, markers, runners, args.timeout_seconds,
                                            args.min_finalized_blocks)
        check_recovery(before, statuses[node])
        print(f"PASS: {node} shadow recovered its PoR projection and {count}-hash prefix")

        if not args.skip_node_restart:
            before = statuses[node]
            command = ["docker", "compose"]
            env_file = REPO / "docker/.env"
            if env_file.is_file():
                command += ["--env-file", str(env_file)]
            command += ["-f", str(REPO / "docker/four-node-cluster.yml"), "restart", node]
            subprocess.run(command, check=True, cwd=REPO)
            markers[node] = status_path(root, node).stat().st_mtime_ns
            statuses, count = wait_for_statuses(root, markers, runners, args.timeout_seconds,
                                                args.min_finalized_blocks)
            check_recovery(before, statuses[node])
            print(f"PASS: {node} node restarted; four shadows still agree on {count} finalized hashes")
    finally:
        for process in runners.values():
            stop_runner(process)
        for log in logs:
            log.close()


if __name__ == "__main__":
    try:
        main()
    except (OSError, RuntimeError, TimeoutError, ValueError, subprocess.CalledProcessError) as error:
        print(f"ERROR: {error}", file=sys.stderr)
        sys.exit(1)
