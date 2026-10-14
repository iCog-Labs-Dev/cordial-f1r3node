#!/usr/bin/env python3
"""Run four persistent PoR observers and check convergence across restarts."""

import argparse
import json
import os
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


def compare_statuses(statuses, min_finalized, min_por_round):
    baseline = statuses[NODES[0]]
    minimum = min(len(statuses[node]["finalized_hashes"]) for node in NODES)
    if minimum < min_finalized or min(statuses[node]["source_height"] for node in NODES) < 1:
        raise ValueError(f"waiting for post-genesis finality ({minimum}/{min_finalized} hashes)")
    if baseline["active_por_round"] < min_por_round:
        raise ValueError(f"waiting for PoR round {min_por_round} (at {baseline['active_por_round']})")
    prefix = baseline["finalized_hashes"][:minimum]
    for node in NODES[1:]:
        status = statuses[node]
        if (status["active_por_round"], status.get("active_por_checkpoint"),
            status["weight_commitment"], status["weights"]) != (
            baseline["active_por_round"], baseline.get("active_por_checkpoint"),
            baseline["weight_commitment"], baseline["weights"]
        ):
            raise ValueError(f"{node} has a different PoR round, checkpoint, commitment, or weights")
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
        if (after.get("active_por_checkpoint"), after["weight_commitment"], after["weights"]) != (
            before.get("active_por_checkpoint"), before["weight_commitment"], before["weights"]
        ):
            raise ValueError("PoR projection changed at the same round after restart")


def wait_for_statuses(root, markers, runners, timeout, min_finalized, min_por_round):
    deadline = time.monotonic() + timeout
    last_error = "waiting for status files"
    while time.monotonic() < deadline:
        for node, process in runners.items():
            if process.poll() is not None:
                raise RuntimeError(f"{node} shadow exited with code {process.returncode}; see {root / node / 'runner.log'}")
        statuses = {node: read_status(status_path(root, node), markers[node]) for node in NODES}
        if all(status is not None for status in statuses.values()):
            try:
                count = compare_statuses(statuses, min_finalized, min_por_round)
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
    parser.add_argument("--min-por-round", type=int, default=0)
    parser.add_argument("--grpc-ports", type=int, nargs=4, default=PORTS)
    parser.add_argument("--rating-keys-dir", type=Path, help="validator-1.key through validator-4.key")
    parser.add_argument("--rating-exchange-dir", type=Path)
    parser.add_argument("--commit-complete-ratings", action="store_true")
    parser.add_argument("--compose-override", type=Path)
    parser.add_argument("--compose-project")
    parser.add_argument("--skip-node-restart", action="store_true", help="only check runner recovery")
    args = parser.parse_args()
    if args.timeout_seconds <= 0 or args.min_finalized_blocks <= 0 or args.min_por_round < 0:
        parser.error("timeouts and minimum finalized blocks must be positive; minimum PoR round must be nonnegative")
    if len(set(args.grpc_ports)) != 4 or any(port < 1 or port > 65535 for port in args.grpc_ports):
        parser.error("--grpc-ports needs four distinct valid TCP ports")
    if args.commit_complete_ratings and args.rating_keys_dir is None:
        parser.error("--commit-complete-ratings requires --rating-keys-dir")
    if args.rating_exchange_dir is not None and args.rating_keys_dir is None:
        parser.error("--rating-exchange-dir requires --rating-keys-dir")
    binary = args.binary.resolve()
    bonds_file = args.bonds_file.resolve()
    root = args.data_root.resolve()
    if not binary.is_file() or not bonds_file.is_file():
        parser.error("build live_por_shadow and provide an existing bonds file first")
    if root == REPO or REPO in root.parents:
        parser.error("--data-root must be outside the repository")
    root.mkdir(parents=True, exist_ok=True)
    keys_dir = args.rating_keys_dir.resolve() if args.rating_keys_dir else None
    exchange_dir = (args.rating_exchange_dir.resolve() if args.rating_exchange_dir
                    else root / "rating-exchange")
    if keys_dir:
        for index in range(1, 5):
            if not (keys_dir / f"validator-{index}.key").is_file():
                parser.error(f"missing validator-{index}.key in {keys_dir}")
    if args.compose_override and not args.compose_override.is_file():
        parser.error("--compose-override must name an existing file")

    runners = {}
    logs = []

    def launch(node, port, index):
        directory = root / node
        directory.mkdir(parents=True, exist_ok=True)
        log = (directory / "runner.log").open("ab")
        logs.append(log)
        command = [str(binary), "--grpc-url", f"http://127.0.0.1:{port}",
                   "--bonds-file", str(bonds_file), "--data-dir", str(directory)]
        if keys_dir:
            command += ["--validator-rating-key-file", str(keys_dir / f"validator-{index}.key"),
                        "--stage-local-ratings", "--rating-exchange-dir", str(exchange_dir)]
        if args.commit_complete_ratings:
            command.append("--commit-complete-ratings")
        runners[node] = subprocess.Popen(
            command,
            stdout=log, stderr=subprocess.STDOUT,
        )

    try:
        markers = {node: status_path(root, node).stat().st_mtime_ns
                   if status_path(root, node).exists() else 0 for node in NODES}
        for index, (node, port) in enumerate(zip(NODES, args.grpc_ports), start=1):
            launch(node, port, index)
        statuses, count = wait_for_statuses(root, markers, runners, args.timeout_seconds,
                                            args.min_finalized_blocks, args.min_por_round)
        print(f"PASS: four shadows agree on round {statuses[NODES[0]]['active_por_round']}, "
              f"checkpoint {statuses[NODES[0]].get('active_por_checkpoint')}, "
              f"weight commitment {statuses[NODES[0]]['weight_commitment']}, "
              f"and {count} finalized hashes")

        node = NODES[0]
        before = statuses[node]
        stop_runner(runners[node])
        markers = {name: 0 for name in NODES}
        markers[node] = status_path(root, node).stat().st_mtime_ns
        launch(node, args.grpc_ports[0], 1)
        statuses, count = wait_for_statuses(root, markers, runners, args.timeout_seconds,
                                            args.min_finalized_blocks, args.min_por_round)
        check_recovery(before, statuses[node])
        print(f"PASS: {node} shadow recovered its PoR projection and {count}-hash prefix")

        if not args.skip_node_restart:
            before = statuses[node]
            command = ["docker", "compose"]
            env_file = REPO / "docker/.env"
            if env_file.is_file():
                command += ["--env-file", str(env_file)]
            command += ["-f", str(REPO / "docker/four-node-cluster.yml")]
            if args.compose_override:
                command += ["-f", str(args.compose_override.resolve())]
            command += ["restart", node]
            environment = os.environ.copy()
            if args.compose_project:
                environment["COMPOSE_PROJECT_NAME"] = args.compose_project
            subprocess.run(command, check=True, cwd=REPO, env=environment)
            markers[node] = status_path(root, node).stat().st_mtime_ns
            statuses, count = wait_for_statuses(root, markers, runners, args.timeout_seconds,
                                                args.min_finalized_blocks, args.min_por_round)
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
