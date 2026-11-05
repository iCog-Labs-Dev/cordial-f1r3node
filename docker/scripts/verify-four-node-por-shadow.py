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


def audit_path(root, node, round_number):
    return root / node / "por" / "checkpoint-audits" / f"round-{round_number:020}.json"


def compare_checkpoint_audits(root, statuses, round_number):
    """Require the same durable rating evidence and candidate on all four observers."""
    if round_number == 0:
        return None
    audits = {}
    for node in NODES:
        path = audit_path(root, node, round_number)
        try:
            audit = json.loads(path.read_text())
        except (FileNotFoundError, json.JSONDecodeError) as error:
            raise ValueError(f"waiting for valid checkpoint audit: {path}") from error
        if audit.get("schema_version") != 1 or audit.get("por_round") != round_number:
            raise ValueError(f"invalid checkpoint audit round: {path}")
        if audit.get("source_prefix_len") != round_number * 16 or len(audit.get("window_block_hashes", [])) != 16:
            raise ValueError(f"invalid checkpoint audit window: {path}")
        hashes = statuses[node]["finalized_hashes"]
        if len(hashes) < round_number * 16 or hashes[(round_number - 1) * 16:round_number * 16] != audit["window_block_hashes"]:
            raise ValueError(f"{node} checkpoint audit differs from its finalized window")
        status = statuses[node]
        if status["active_por_round"] == round_number:
            if (audit["checkpoint_hash"], audit["weight_commitment"], audit["weights"]) != (
                status.get("active_por_checkpoint"), status["weight_commitment"], status["weights"]
            ):
                raise ValueError(f"{node} checkpoint audit differs from its active PoR projection")
        elif (status.get("candidate") or {}).get("por_round") == round_number:
            candidate = status["candidate"]
            if (audit["source_prefix_hash"], audit["checkpoint_hash"],
                audit["weight_commitment"], audit["weights"]) != (
                candidate["source"]["prefix_hash"], candidate["checkpoint_hash"],
                candidate["weight_commitment"], candidate["weights"]
            ):
                raise ValueError(f"{node} checkpoint audit differs from its PoR candidate")
        audits[node] = audit
    baseline = audits[NODES[0]]
    for node in NODES[1:]:
        if audits[node] != baseline:
            raise ValueError(f"{node} has a different checkpoint rating audit")
    return baseline


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


def compare_statuses(statuses, min_finalized, min_por_round=0, min_candidate_round=0,
                     min_distinct_weights=0, require_ejected_validator=None,
                     min_post_cutover_waves=0):
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
            status["weight_commitment"], status["weights"],
            status.get("active_effective_wave"), status.get("ejected_keys", []),
            status.get("retained_reputation", {})) != (
            baseline["active_por_round"], baseline.get("active_por_checkpoint"),
            baseline["weight_commitment"], baseline["weights"],
            baseline.get("active_effective_wave"), baseline.get("ejected_keys", []),
            baseline.get("retained_reputation", {})
        ):
            raise ValueError(f"{node} has a different PoR round, checkpoint, commitment, weights, or penalty state")
        if status["finalized_hashes"][:minimum] != prefix:
            raise ValueError(f"{node} has a different finalized prefix")
        if len(status["finalized_hashes"]) == len(baseline["finalized_hashes"]):
            if status["finalized_anchor"] != baseline["finalized_anchor"]:
                raise ValueError(f"{node} has a different finalized anchor")
    if min_candidate_round and baseline["active_por_round"] < min_candidate_round:
        candidate = baseline.get("candidate")
        if candidate is None or candidate.get("por_round", 0) < min_candidate_round:
            raise ValueError(f"waiting for complete PoR candidate round {min_candidate_round}")
        for node in NODES[1:]:
            if statuses[node].get("candidate") != candidate:
                raise ValueError(f"{node} has a different PoR checkpoint candidate")
    if min_distinct_weights and len(set(baseline["weights"].values())) < min_distinct_weights:
        raise ValueError(f"waiting for {min_distinct_weights} distinct active PoR weights")
    if require_ejected_validator is not None:
        offender = statuses[NODES[require_ejected_validator - 1]].get("configured_rating_rater")
        if offender is None:
            raise ValueError("waiting for the offending validator's configured rating identity")
        if (offender not in baseline.get("ejected_keys", []) or
                offender not in baseline.get("retained_reputation", {}) or
                baseline["weights"].get(offender) != 0):
            raise ValueError(f"waiting for validator {require_ejected_validator} PoR ejection and zero weight")
    if min_post_cutover_waves:
        effective_wave = baseline.get("active_effective_wave")
        if effective_wave is None:
            raise ValueError("waiting for an active future PoR weight epoch")
        target_wave = effective_wave + min_post_cutover_waves - 1
        for node in NODES:
            observed = statuses[node].get("observed_finalized_wave")
            if observed is None or observed < target_wave:
                raise ValueError(f"waiting for {node} to finalize PoR wave {target_wave}")
    return minimum


def check_progress(before, after, minimum_new_blocks=1):
    """State preservation alone is not evidence that consensus resumed."""
    check_recovery(before, after)
    growth = len(after["finalized_hashes"]) - len(before["finalized_hashes"])
    if growth < minimum_new_blocks:
        raise ValueError(f"waiting for post-restart finality ({growth}/{minimum_new_blocks} new blocks)")


def check_recovery(before, after):
    if after["active_por_round"] < before["active_por_round"]:
        raise ValueError("PoR round regressed after restart")
    if after["finalized_hashes"][:len(before["finalized_hashes"])] != before["finalized_hashes"]:
        raise ValueError("finalized prefix changed or shrank after restart")
    if not set(before.get("ejected_keys", [])).issubset(after.get("ejected_keys", [])):
        raise ValueError("PoR ejection disappeared after restart")
    for key, retained in before.get("retained_reputation", {}).items():
        if after.get("retained_reputation", {}).get(key) != retained:
            raise ValueError("PoR retained reputation changed after restart")
    if after["active_por_round"] == before["active_por_round"]:
        if (after.get("active_por_checkpoint"), after["weight_commitment"], after["weights"],
            after.get("active_effective_wave"), after.get("ejected_keys", []),
            after.get("retained_reputation", {})) != (
            before.get("active_por_checkpoint"), before["weight_commitment"], before["weights"],
            before.get("active_effective_wave"), before.get("ejected_keys", []),
            before.get("retained_reputation", {})
        ):
            raise ValueError("PoR projection changed at the same round after restart")
    candidate = before.get("candidate")
    if candidate is not None:
        if after["active_por_round"] < candidate["por_round"]:
            if after.get("candidate") != candidate:
                raise ValueError("PoR checkpoint candidate changed or disappeared after restart")
        elif after["active_por_round"] == candidate["por_round"]:
            if (after.get("active_por_checkpoint"), after["weight_commitment"], after["weights"]) != (
                candidate["checkpoint_hash"], candidate["weight_commitment"], candidate["weights"]
            ):
                raise ValueError("activated PoR projection differs from the prior candidate")


def wait_for_statuses(root, markers, runners, timeout, min_finalized, min_por_round,
                      min_candidate_round, progress_before=None, minimum_new_blocks=1,
                      min_distinct_weights=0, require_ejected_validator=None,
                      min_post_cutover_waves=0, checkpoint_ratings=False):
    deadline = time.monotonic() + timeout
    last_error = "waiting for status files"
    while time.monotonic() < deadline:
        for node, process in runners.items():
            if process.poll() is not None:
                raise RuntimeError(f"{node} shadow exited with code {process.returncode}; see {root / node / 'runner.log'}")
        statuses = {node: read_status(status_path(root, node), markers[node]) for node in NODES}
        if all(status is not None for status in statuses.values()):
            try:
                count = compare_statuses(
                    statuses, min_finalized, min_por_round, min_candidate_round,
                    min_distinct_weights, require_ejected_validator, min_post_cutover_waves)
                if progress_before is not None:
                    for node in NODES:
                        check_progress(progress_before[node], statuses[node], minimum_new_blocks)
                if checkpoint_ratings:
                    audit_round = max(min_por_round, min_candidate_round)
                    if audit_round == 0:
                        audit_round = statuses[NODES[0]]["active_por_round"]
                    compare_checkpoint_audits(root, statuses, audit_round)
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


def write_evidence(directory, label, statuses, audit=None):
    destination = directory / f"{label}.json"
    destination.write_text(json.dumps({
        "captured_at_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "statuses": statuses,
        "checkpoint_rating_audit": audit,
    }, indent=2, sort_keys=True) + "\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--data-root", type=Path, required=True, help="persistent directory outside the repository")
    parser.add_argument("--binary", type=Path, default=REPO / "target/debug/live_por_shadow")
    parser.add_argument("--bonds-file", type=Path, default=REPO / "docker/genesis/cordial-bonds.txt")
    parser.add_argument("--timeout-seconds", type=int, default=300)
    parser.add_argument("--min-finalized-blocks", type=int, default=2)
    parser.add_argument("--min-por-round", type=int, default=0)
    parser.add_argument("--min-new-finalized-blocks", type=int, default=1,
                        help="required finalized growth on every observer after each restart")
    parser.add_argument("--min-candidate-round", type=int, default=0,
                        help="accept an agreed complete checkpoint candidate before activation")
    parser.add_argument("--min-distinct-weights", type=int, default=0,
                        help="require this many distinct active PoR weight values")
    parser.add_argument("--require-ejected-validator", type=int, choices=range(1, 5),
                        help="require this validator's rating key to be ejected with zero active weight")
    parser.add_argument("--min-post-cutover-waves", type=int, default=0,
                        help="require this many finalized Cordial waves from the active effective wave")
    parser.add_argument("--grpc-ports", type=int, nargs=4, default=PORTS)
    parser.add_argument("--rating-keys-dir", type=Path, help="validator-1.key through validator-4.key")
    parser.add_argument("--rating-exchange-dir", type=Path)
    parser.add_argument("--commit-complete-ratings", action="store_true")
    parser.add_argument("--checkpoint-ratings", action="store_true",
                        help="use v2 finalized-prefix checkpoint rounds in all observers")
    parser.add_argument("--checkpoint-epoch-delay-waves", type=int,
                        help="fixed source-to-cutover delay for one future-only PoR epoch")
    parser.add_argument("--compose-override", type=Path)
    parser.add_argument("--compose-project")
    parser.add_argument("--skip-node-restart", action="store_true", help="only check runner recovery")
    args = parser.parse_args()
    if (args.timeout_seconds <= 0 or args.min_finalized_blocks <= 0 or
            args.min_por_round < 0 or args.min_candidate_round < 0 or
            args.min_new_finalized_blocks < 1 or args.min_distinct_weights < 0 or
            args.min_post_cutover_waves < 0):
        parser.error("timeouts and minimum finalized blocks must be positive; minimum PoR rounds must be nonnegative")
    if len(set(args.grpc_ports)) != 4 or any(port < 1 or port > 65535 for port in args.grpc_ports):
        parser.error("--grpc-ports needs four distinct valid TCP ports")
    if args.commit_complete_ratings and args.rating_keys_dir is None:
        parser.error("--commit-complete-ratings requires --rating-keys-dir")
    if args.rating_exchange_dir is not None and args.rating_keys_dir is None:
        parser.error("--rating-exchange-dir requires --rating-keys-dir")
    if args.checkpoint_ratings and args.rating_keys_dir is None:
        parser.error("--checkpoint-ratings requires --rating-keys-dir")
    if args.min_candidate_round and not args.checkpoint_ratings:
        parser.error("--min-candidate-round requires --checkpoint-ratings")
    if args.require_ejected_validator and not args.rating_keys_dir:
        parser.error("--require-ejected-validator requires --rating-keys-dir")
    if args.min_post_cutover_waves and args.checkpoint_epoch_delay_waves is None:
        parser.error("--min-post-cutover-waves requires --checkpoint-epoch-delay-waves")
    if args.checkpoint_epoch_delay_waves is not None:
        if args.checkpoint_epoch_delay_waves <= 0:
            parser.error("--checkpoint-epoch-delay-waves must be positive")
        if not (args.checkpoint_ratings and args.commit_complete_ratings):
            parser.error("--checkpoint-epoch-delay-waves requires committed checkpoint ratings")
    binary = args.binary.resolve()
    bonds_file = args.bonds_file.resolve()
    root = args.data_root.resolve()
    if not binary.is_file() or not bonds_file.is_file():
        parser.error("build live_por_shadow and provide an existing bonds file first")
    if root == REPO or REPO in root.parents:
        parser.error("--data-root must be outside the repository")
    root.mkdir(parents=True, exist_ok=True)
    evidence_dir = root / "evidence" / f"{time.strftime('%Y%m%dT%H%M%SZ', time.gmtime())}-{os.getpid()}"
    evidence_dir.mkdir(parents=True)
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
        if args.checkpoint_ratings:
            command.append("--checkpoint-ratings")
        if args.checkpoint_epoch_delay_waves is not None:
            command += ["--checkpoint-epoch-delay-waves", str(args.checkpoint_epoch_delay_waves)]
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
                                            args.min_finalized_blocks, args.min_por_round,
                                            args.min_candidate_round,
                                            min_distinct_weights=args.min_distinct_weights,
                                            require_ejected_validator=args.require_ejected_validator,
                                            min_post_cutover_waves=args.min_post_cutover_waves,
                                            checkpoint_ratings=args.checkpoint_ratings)
        audit_round = max(args.min_por_round, args.min_candidate_round)
        if audit_round == 0:
            audit_round = statuses[NODES[0]]["active_por_round"]
        audit = compare_checkpoint_audits(root, statuses, audit_round) if args.checkpoint_ratings else None
        write_evidence(evidence_dir, "initial", statuses, audit)
        print(f"PASS: four shadows agree on round {statuses[NODES[0]]['active_por_round']}, "
              f"checkpoint {statuses[NODES[0]].get('active_por_checkpoint')}, "
              f"weight commitment {statuses[NODES[0]]['weight_commitment']}, "
              f"and {count} finalized hashes")
        if args.require_ejected_validator:
            offender = statuses[NODES[args.require_ejected_validator - 1]]["configured_rating_rater"]
            print(f"PASS: validator {args.require_ejected_validator} is ejected with zero PoR weight "
                  f"and retained reputation {statuses[NODES[0]]['retained_reputation'][offender]}")
        if args.min_candidate_round:
            candidate = statuses[NODES[0]].get("candidate")
            if statuses[NODES[0]]["active_por_round"] < args.min_candidate_round:
                print(f"PASS: four shadows agree on complete candidate round {candidate['por_round']}, "
                      f"checkpoint {candidate['checkpoint_hash']}, and "
                      f"weight commitment {candidate['weight_commitment']}")
            else:
                print(f"PASS: four shadows already activated PoR round "
                      f"{statuses[NODES[0]]['active_por_round']}")

        node = NODES[0]
        before = statuses[node]
        progress_before = statuses
        stop_runner(runners[node])
        markers = {name: 0 for name in NODES}
        markers[node] = status_path(root, node).stat().st_mtime_ns
        launch(node, args.grpc_ports[0], 1)
        statuses, count = wait_for_statuses(root, markers, runners, args.timeout_seconds,
                                            args.min_finalized_blocks, args.min_por_round,
                                            args.min_candidate_round,
                                            progress_before, args.min_new_finalized_blocks,
                                            args.min_distinct_weights, args.require_ejected_validator,
                                            args.min_post_cutover_waves, args.checkpoint_ratings)
        check_progress(before, statuses[node], args.min_new_finalized_blocks)
        audit = compare_checkpoint_audits(root, statuses, audit_round) if args.checkpoint_ratings else None
        write_evidence(evidence_dir, "observer-restart", statuses, audit)
        print(f"PASS: {node} shadow recovered its PoR projection and {count}-hash prefix")

        if not args.skip_node_restart:
            before = statuses[node]
            progress_before = statuses
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
                                                args.min_finalized_blocks, args.min_por_round,
                                                args.min_candidate_round,
                                                progress_before, args.min_new_finalized_blocks,
                                                args.min_distinct_weights, args.require_ejected_validator,
                                                args.min_post_cutover_waves, args.checkpoint_ratings)
            check_progress(before, statuses[node], args.min_new_finalized_blocks)
            audit = compare_checkpoint_audits(root, statuses, audit_round) if args.checkpoint_ratings else None
            write_evidence(evidence_dir, "validator-restart", statuses, audit)
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
