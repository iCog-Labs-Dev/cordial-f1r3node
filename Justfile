set shell := ["bash", "-eu", "-o", "pipefail", "-c"]

toolchain := "nightly-2025-06-15"
docker_env := "docker/.env"
docker_standalone := "docker/standalone.yml"
docker_prebuilt_standalone := "docker/prebuilt-standalone.yml"
docker_conformance := "docker/conformance.yml"
docker_four_node := "docker/four-node-intercept.yml"
docker_four_node_cluster := "docker/four-node-cluster.yml"
demo_data := "/tmp/cordial-f1r3node-demo"

default:
    just --list

fmt:
    cargo +{{toolchain}} fmt -p cordial-miners-core -p cordial-f1r3node-adapter -p cordial-f1r3space-adapter

clippy:
    cargo +{{toolchain}} clippy -p cordial-miners-core -p cordial-f1r3node-adapter -p cordial-f1r3space-adapter --all-targets --all-features --no-deps -- -D warnings

build:
    cargo +{{toolchain}} build --workspace

test:
    cargo +{{toolchain}} test --workspace

test-core:
    cargo +{{toolchain}} test -p cordial-miners-core

test-adapter:
    cargo +{{toolchain}} test -p cordial-f1r3node-adapter

test-consensus-flag:
    cargo +{{toolchain}} test -p cordial-f1r3node-adapter parses_consensus_flag_for_cordial_miners -- --exact --nocapture

test-cordial-startup:
    cargo +{{toolchain}} test -p cordial-f1r3node-adapter startup_with_cordial_mode_returns_cordial_stub -- --exact --nocapture

check-core-boundaries:
    ./scripts/check_core_boundaries.sh

ci:
    just fmt
    just clippy
    just build
    just test
    just check-core-boundaries

demo-cordial-env:
    cp -n docker/.env.example {{docker_env}}

demo-cordial-config: demo-cordial-env
    docker compose --env-file {{docker_env}} -f {{docker_standalone}} config >/dev/null
    docker compose --env-file {{docker_env}} -f {{docker_prebuilt_standalone}} config >/dev/null
    docker compose --env-file {{docker_env}} -f {{docker_conformance}} config >/dev/null
    docker compose --env-file {{docker_env}} -f {{docker_four_node}} config >/dev/null
    docker compose --env-file {{docker_env}} -f {{docker_four_node_cluster}} config >/dev/null
    echo "Cordial Miners compose files are valid."

demo-cordial-image-check: demo-cordial-env
    image="$(grep '^CORDIAL_F1R3NODE_IMAGE=' {{docker_env}} | cut -d= -f2-)"; help="$(docker run --rm "$image" run --help 2>&1)"; grep -q 'cordial-miners' <<<"$help"; echo "$image supports --consensus cordial-miners"

demo-cordial-build: demo-cordial-env
    docker compose --env-file {{docker_env}} -f {{docker_standalone}} build cordial-standalone

demo-cordial-conformance: demo-cordial-env
    docker compose --env-file {{docker_env}} -f {{docker_conformance}} run --rm cordial-conformance

demo-cordial-conformance-local:
    cargo test -p cordial-f1r3node-adapter --test conformance

demo-cordial-up: demo-cordial-env
    docker compose --env-file {{docker_env}} -f {{docker_standalone}} up -d --build

demo-cordial-up-prebuilt: demo-cordial-env demo-cordial-image-check
    docker compose --env-file {{docker_env}} -f {{docker_prebuilt_standalone}} up -d

demo-cordial-wait:
    timeout 180 bash -c 'until curl -fsS http://127.0.0.1:40403/api/status >/dev/null; do sleep 5; done'
    curl -s http://127.0.0.1:40403/api/status | jq

demo-cordial-status:
    curl -s http://127.0.0.1:40403/api/status | jq

demo-cordial-propose:
    curl -s -X POST http://127.0.0.1:40405/api/propose

demo-cordial-blocks:
    curl -s http://127.0.0.1:40403/api/blocks/10 | jq

demo-cordial-logs:
    docker compose --env-file {{docker_env}} -f {{docker_standalone}} logs --tail=160 cordial-standalone

demo-cordial-smoke: demo-cordial-status demo-cordial-propose demo-cordial-blocks

demo-cordial-four-node-config: demo-cordial-env
    docker compose --env-file {{docker_env}} -f {{docker_four_node}} config >/dev/null
    echo "Cordial Miners four-node compose file is valid."

demo-cordial-four-node-up: demo-cordial-env demo-cordial-image-check
    docker compose --env-file {{docker_env}} -f {{docker_four_node}} up -d

demo-cordial-four-node-wait:
    timeout 240 bash -c 'for port in 51403 52403 53403 54403; do until curl -fsS "http://127.0.0.1:${port}/api/status" >/dev/null; do sleep 5; done; done'

demo-cordial-four-node-status:
    for port in 51403 52403 53403 54403; do echo "node http:${port}"; curl -s "http://127.0.0.1:${port}/api/status" | jq '{networkId,isValidator,isReady,peers,nodes,lastFinalizedBlockNumber}'; done

demo-cordial-four-node-verify: demo-cordial-four-node-up
    docker compose --env-file {{docker_env}} -f {{docker_four_node}} run --rm cordial-four-node-verifier

demo-cordial-four-node-blocks:
    for port in 51403 52403 53403 54403; do echo "node http:${port}"; curl -s "http://127.0.0.1:${port}/api/blocks/10" | jq '[.[].blockInfo | {blockNumber,blockHash,sender,seqNum,deployCount,isFinalized}]'; done

demo-cordial-four-node-logs: demo-cordial-env
    docker compose --env-file {{docker_env}} -f {{docker_four_node}} logs --tail=120 cordial-node-1 cordial-node-2 cordial-node-3 cordial-node-4

demo-cordial-four-node-down: demo-cordial-env
    docker compose --env-file {{docker_env}} -f {{docker_four_node}} down -v

demo-cordial-four-node-cluster-config: demo-cordial-env
    docker compose --env-file {{docker_env}} -f {{docker_four_node_cluster}} config >/dev/null
    echo "Cordial Miners real four-node cluster compose file is valid."

demo-cordial-four-node-cluster-up: demo-cordial-env demo-cordial-image-check
    docker compose --env-file {{docker_env}} -f {{docker_four_node_cluster}} up -d

demo-cordial-four-node-cluster-up-legacy: demo-cordial-env
    docker-compose --env-file {{docker_env}} -f {{docker_four_node_cluster}} up

demo-cordial-four-node-cluster-wait:
    timeout 300 bash -c 'for port in 55403 51403 52403 53403 54403; do until curl -fsS "http://127.0.0.1:${port}/api/status" >/dev/null; do sleep 5; done; done'

demo-cordial-four-node-cluster-status:
    for port in 55403 51403 52403 53403 54403; do echo "node http:${port}"; curl -s "http://127.0.0.1:${port}/api/status" | jq '{networkId,isValidator,isReady,peers,nodes,lastFinalizedBlockNumber}'; done

demo-cordial-four-node-cluster-verify: demo-cordial-four-node-cluster-up
    docker compose --env-file {{docker_env}} -f {{docker_four_node_cluster}} run --rm cordial-four-node-cluster-verifier

demo-cordial-four-node-cluster-ordering:
    ./docker/scripts/verify-four-node-cluster-ordering.sh

demo-cordial-four-node-cluster-blocks:
    for port in 51403 52403 53403 54403; do echo "node http:${port}"; curl -s "http://127.0.0.1:${port}/api/blocks/10" | jq '[.[].blockInfo | {blockNumber,blockHash,sender,seqNum,deployCount,isFinalized}]'; done

demo-cordial-four-node-cluster-logs: demo-cordial-env
    docker compose --env-file {{docker_env}} -f {{docker_four_node_cluster}} logs --tail=120 cordial-boot cordial-validator-1 cordial-validator-2 cordial-validator-3 cordial-validator-4

demo-cordial-four-node-cluster-down: demo-cordial-env
    docker compose --env-file {{docker_env}} -f {{docker_four_node_cluster}} down -v

demo-cordial-four-node-cluster-down-legacy: demo-cordial-env
    docker-compose --env-file {{docker_env}} -f {{docker_four_node_cluster}} down -v

demo-cordial-down: demo-cordial-env
    docker compose --env-file {{docker_env}} -f {{docker_standalone}} down -v
    docker compose --env-file {{docker_env}} -f {{docker_prebuilt_standalone}} down -v
    docker compose --env-file {{docker_env}} -f {{docker_conformance}} down -v
    docker compose --env-file {{docker_env}} -f {{docker_four_node}} down -v
    docker compose --env-file {{docker_env}} -f {{docker_four_node_cluster}} down -v

demo-cordial-local-clean:
    rm -rf {{demo_data}}

demo-cordial-local-node:
    ../f1r3node/target/debug/node run -s --host 127.0.0.1 --api-host 127.0.0.1 --network-id cordial-demo --data-dir {{demo_data}} --bonds-file docker/genesis/cordial-bonds.txt --wallets-file docker/genesis/cordial-wallets.txt --validator-private-key 0101010101010101010101010101010101010101010101010101010101010101 --allow-private-addresses --no-upnp --consensus cordial-miners --native-token-name F1R3CAP --native-token-symbol F1R3 --native-token-decimals 8
