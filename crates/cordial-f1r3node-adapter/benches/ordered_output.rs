//! Single-threaded finalized-output operations. No lock contention is measured.
//! Update includes validation and replacement of the old prefix; fixture cloning
//! and destruction of the final container are excluded. Construction includes
//! ownership of block identities and the production timestamp/normalization.
use cordial_f1r3node_adapter::ordered_output::OrderedFinalizedOutput;
use cordial_f1r3node_adapter::shared_ordered_output::{ReadOrderedOutput, SharedOrderedOutput};
use cordial_miners_core::bench_support as support;
use criterion::{BatchSize, BenchmarkId, Criterion, criterion_group, criterion_main};
use std::time::Duration;

fn output(n: usize) -> OrderedFinalizedOutput {
    let ids: Vec<_> = support::chain(n, 8)
        .into_iter()
        .map(|b| b.identity)
        .collect();
    let anchor = ids.last().cloned();
    OrderedFinalizedOutput::new(ids, anchor, 3, 4, n).with_timestamp(0)
}
fn bench_output(c: &mut Criterion) {
    let mut group = c.benchmark_group("ordered_output");
    group.sample_size(20);
    group.measurement_time(Duration::from_secs(2));
    for n in support::sizes(&[100, 1_000], &[100, 1_000, 10_000]) {
        let previous = output(n);
        let next = output(n + 10);
        assert!(next.preserves_prefix(&previous));
        let mut checked = SharedOrderedOutput::from_output(previous.clone());
        checked.update(next.clone()).expect("append valid");
        assert_eq!(checked.latest().expect("published").blocks, next.blocks);
        let mut invalid = next.clone();
        invalid.blocks.swap(0, 1);
        assert!(checked.update(invalid).is_err(), "reject rewritten prefix");

        for (name, prefix) in [
            ("publish_from_empty", OrderedFinalizedOutput::default()),
            ("append_10", previous.clone()),
        ] {
            group.bench_function(BenchmarkId::new(name, n), |b| {
                b.iter_batched_ref(
                    || {
                        (
                            SharedOrderedOutput::from_output(prefix.clone()),
                            Some(next.clone()),
                        )
                    },
                    |(shared, value)| {
                        shared
                            .update(value.take().expect("fresh output"))
                            .expect("publish")
                    },
                    BatchSize::SmallInput,
                );
            });
        }
        let shared = SharedOrderedOutput::from_output(previous.clone());
        assert_eq!(shared.latest().expect("latest").blocks.len(), n);
        group.bench_function(BenchmarkId::new("latest_borrow", n), |b| {
            b.iter(|| shared.latest())
        });
        let json = serde_json::to_string(&previous).expect("encode");
        let decoded: OrderedFinalizedOutput = serde_json::from_str(&json).expect("decode");
        assert_eq!(decoded.blocks, previous.blocks);
        group.bench_function(BenchmarkId::new("json_serialise", n), |b| {
            b.iter(|| serde_json::to_string(&previous).expect("encode"))
        });
        assert_eq!(previous.block_hashes().len(), n);
        group.bench_function(BenchmarkId::new("block_hashes", n), |b| {
            b.iter(|| previous.block_hashes())
        });
        group.bench_function(BenchmarkId::new("construction", n), |b| {
            b.iter_batched(
                || previous.blocks.clone(),
                |blocks| {
                    let anchor = blocks.last().cloned();
                    OrderedFinalizedOutput::new(blocks, anchor, 3, 4, n)
                },
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}
// Equal payload/predecessor bytes must remain separate in snapshot indexes
// when produced by different validators. Cryptography and graph assembly are
// excluded; build_snapshot includes decoding, indexes, ordering and translation.
fn bench_snapshot_identity(c: &mut Criterion) {
    use cordial_f1r3node_adapter::snapshot::{CasperShardConf, build_snapshot};
    use cordial_miners_core::{
        Block, BlockContent, BlockIdentity,
        crypto::block_hash,
        execution::{Bond, CordialBlockPayload},
    };
    use std::collections::HashSet;

    let mut group = c.benchmark_group("snapshot/equal_content_creators");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(2));
    for layers in support::sizes(&[6, 18], &[6, 18, 60]) {
        let weights = support::bonds(4);
        let bonds: Vec<_> = (0..4)
            .map(|index| Bond {
                validator: support::node(index),
                stake: 100,
            })
            .collect();
        let mut blocks = Vec::new();
        let mut predecessors = HashSet::new();
        for height in 0..layers {
            let mut payload = CordialBlockPayload::genesis(bonds.clone());
            payload.state.block_number = height as u64;
            let content = BlockContent {
                payload: payload.to_bytes(),
                predecessors,
            };
            let mut next = HashSet::new();
            for index in 0..4 {
                let creator = support::node(index);
                let block = Block {
                    identity: BlockIdentity {
                        content_hash: block_hash(&creator, &content),
                        creator,
                        signature: vec![],
                    },
                    content: content.clone(),
                };
                next.insert(block.identity.clone());
                blocks.push(block);
            }
            assert_eq!(next.len(), 4, "creator-bound identities stay distinct");
            predecessors = next;
        }
        let dag = support::insert_all(&blocks);
        let make = || {
            build_snapshot(&dag, &weights, CasperShardConf::default(), "root").expect("snapshot")
        };
        let checked = make();
        assert_eq!(checked.dag.dag_set.len(), layers * 4);
        assert_eq!(checked.dag.block_number_map.len(), layers * 4);
        assert_eq!(checked.dag.height_map.len(), layers);
        assert_eq!(checked.dag.latest_messages_map.len(), 4);
        for (height, layer) in blocks.chunks(4).enumerate() {
            let expected: HashSet<_> = layer
                .iter()
                .map(|block| block.identity.content_hash.to_vec())
                .collect();
            assert_eq!(expected.len(), 4);
            assert_eq!(
                checked.dag.height_map.get(&(height as i64)),
                Some(&expected)
            );
            for block in layer {
                support::assert_content_eq(&block.content, &layer[0].content);
                let hash = block.identity.content_hash.to_vec();
                assert!(checked.dag.dag_set.contains(&hash));
                assert_eq!(checked.dag.block_number_map[&hash], height as i64);
                if height + 1 < layers {
                    let children: HashSet<_> = blocks[(height + 1) * 4..(height + 2) * 4]
                        .iter()
                        .map(|child| child.identity.content_hash.to_vec())
                        .collect();
                    assert_eq!(checked.dag.child_map.get(&hash), Some(&children));
                } else {
                    assert_eq!(
                        checked
                            .dag
                            .latest_messages_map
                            .get(&block.identity.creator.0),
                        Some(&hash)
                    );
                }
            }
        }
        group.bench_function(BenchmarkId::from_parameter(layers * 4), |b| b.iter(make));
    }
    group.finish();
}
criterion_group!(benches, bench_output, bench_snapshot_identity);
criterion_main!(benches);
