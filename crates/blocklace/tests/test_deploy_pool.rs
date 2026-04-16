use std::collections::HashSet;
use blocklace::blocklace::Blocklace;
use blocklace::crypto::hash_content;
use blocklace::execution::{
    DeployPool, DeployPoolConfig, PoolError, compute_deploys_in_scope,
    CordialBlockPayload, BlockState, Bond, Deploy, SignedDeploy,
    ProcessedDeploy,
};
use blocklace::{Block, BlockContent, BlockIdentity, NodeId};

// ── Helpers ──

fn node(id: u8) -> NodeId {
    NodeId(vec![id])
}

fn make_deploy(sig_byte: u8, valid_after: u64, timestamp: u64, phlo_price: u64) -> SignedDeploy {
    SignedDeploy {
        deploy: Deploy {
            term: format!("term-{}", sig_byte).into_bytes(),
            timestamp,
            phlo_price,
            phlo_limit: 10_000,
            valid_after_block_number: valid_after,
            shard_id: "root".to_string(),
        },
        deployer: vec![sig_byte; 32],
        signature: vec![sig_byte; 64],
    }
}

fn default_pool() -> DeployPool {
    DeployPool::new(DeployPoolConfig::default())
}

// ── Add / remove ──

#[test]
fn add_deploy_to_empty_pool() {
    let mut pool = default_pool();
    let deploy = make_deploy(1, 0, 1000, 1);
    assert!(pool.add(deploy).is_ok());
    assert_eq!(pool.len(), 1);
}

#[test]
fn add_duplicate_signature_fails() {
    let mut pool = default_pool();
    let d = make_deploy(1, 0, 1000, 1);
    pool.add(d.clone()).unwrap();
    assert_eq!(pool.add(d), Err(PoolError::Duplicate));
    assert_eq!(pool.len(), 1);
}

#[test]
fn add_with_empty_signature_fails() {
    let mut pool = default_pool();
    let mut d = make_deploy(1, 0, 1000, 1);
    d.signature = vec![];
    assert_eq!(pool.add(d), Err(PoolError::InvalidSignature));
}

#[test]
fn add_below_min_phlo_price_fails() {
    let config = DeployPoolConfig {
        min_phlo_price: 10,
        ..Default::default()
    };
    let mut pool = DeployPool::new(config);
    let d = make_deploy(1, 0, 1000, 5); // below min
    assert!(matches!(
        pool.add(d),
        Err(PoolError::InsufficientPhloPrice { required: 10, actual: 5 })
    ));
}

#[test]
fn remove_by_signature() {
    let mut pool = default_pool();
    let d = make_deploy(1, 0, 1000, 1);
    let sig = d.signature.clone();
    pool.add(d).unwrap();
    assert!(pool.remove(&sig));
    assert!(pool.is_empty());
    assert!(!pool.remove(&sig)); // second time: not present
}

// ── Selection filters ──

#[test]
fn select_filters_out_future_deploys() {
    let mut pool = default_pool();
    // Deploy valid only after block 100 — not yet
    pool.add(make_deploy(1, 100, 1000, 1)).unwrap();
    let selected = pool.select_for_block(50, 0, &HashSet::new());
    assert!(selected.deploys.is_empty());
}

#[test]
fn select_filters_out_block_expired_deploys() {
    let config = DeployPoolConfig {
        deploy_lifespan: 50,
        ..Default::default()
    };
    let mut pool = DeployPool::new(config);
    // Current block = 200, lifespan = 50, so earliest = 150
    // Deploy with valid_after = 100 is expired (100 <= 150)
    pool.add(make_deploy(1, 100, 1000, 1)).unwrap();
    let selected = pool.select_for_block(200, 0, &HashSet::new());
    assert!(selected.deploys.is_empty());
}

#[test]
fn select_filters_out_duplicated_deploys() {
    let mut pool = default_pool();
    let d = make_deploy(1, 0, 1000, 1);
    let sig = d.signature.clone();
    pool.add(d).unwrap();

    let mut in_scope = HashSet::new();
    in_scope.insert(sig);

    let selected = pool.select_for_block(5, 0, &in_scope);
    assert!(selected.deploys.is_empty());
}

#[test]
fn select_includes_valid_deploys() {
    let mut pool = default_pool();
    pool.add(make_deploy(1, 0, 1000, 1)).unwrap();
    pool.add(make_deploy(2, 0, 2000, 1)).unwrap();
    let selected = pool.select_for_block(5, 0, &HashSet::new());
    assert_eq!(selected.deploys.len(), 2);
    assert!(!selected.cap_hit);
}

// ── Capping ──

#[test]
fn select_caps_at_max_user_deploys() {
    let config = DeployPoolConfig {
        max_user_deploys_per_block: 3,
        ..Default::default()
    };
    let mut pool = DeployPool::new(config);
    // Add 5 valid deploys
    for i in 1..=5u8 {
        pool.add(make_deploy(i, 0, i as u64 * 1000, 1)).unwrap();
    }
    let selected = pool.select_for_block(10, 0, &HashSet::new());
    assert_eq!(selected.deploys.len(), 3);
    assert!(selected.cap_hit);
}

#[test]
fn cap_selects_oldest_plus_newest() {
    let config = DeployPoolConfig {
        max_user_deploys_per_block: 3,
        ..Default::default()
    };
    let mut pool = DeployPool::new(config);
    // Deploys with distinct timestamps
    pool.add(make_deploy(1, 0, 1000, 1)).unwrap(); // oldest
    pool.add(make_deploy(2, 0, 2000, 1)).unwrap();
    pool.add(make_deploy(3, 0, 3000, 1)).unwrap();
    pool.add(make_deploy(4, 0, 4000, 1)).unwrap();
    pool.add(make_deploy(5, 0, 5000, 1)).unwrap(); // newest

    let selected = pool.select_for_block(10, 0, &HashSet::new());
    assert_eq!(selected.deploys.len(), 3);

    // Should contain the two oldest (timestamps 1000, 2000) + the newest (5000)
    let timestamps: HashSet<u64> = selected.deploys.iter().map(|d| d.deploy.timestamp).collect();
    assert!(timestamps.contains(&1000));
    assert!(timestamps.contains(&2000));
    assert!(timestamps.contains(&5000));
}

#[test]
fn cap_of_one_selects_newest() {
    let config = DeployPoolConfig {
        max_user_deploys_per_block: 1,
        ..Default::default()
    };
    let mut pool = DeployPool::new(config);
    pool.add(make_deploy(1, 0, 1000, 1)).unwrap();
    pool.add(make_deploy(2, 0, 2000, 1)).unwrap();
    pool.add(make_deploy(3, 0, 3000, 1)).unwrap();

    let selected = pool.select_for_block(10, 0, &HashSet::new());
    assert_eq!(selected.deploys.len(), 1);
    assert_eq!(selected.deploys[0].deploy.timestamp, 3000);
    assert!(selected.cap_hit);
}

// ── Pruning ──

#[test]
fn prune_removes_block_expired() {
    let config = DeployPoolConfig {
        deploy_lifespan: 50,
        ..Default::default()
    };
    let mut pool = DeployPool::new(config);
    pool.add(make_deploy(1, 100, 1000, 1)).unwrap(); // expired at block 200
    pool.add(make_deploy(2, 180, 2000, 1)).unwrap(); // still valid

    let removed = pool.prune_expired(200, 0);
    assert_eq!(removed.len(), 1);
    assert_eq!(pool.len(), 1);
}

#[test]
fn prune_does_nothing_if_all_valid() {
    let mut pool = default_pool();
    pool.add(make_deploy(1, 0, 1000, 1)).unwrap();
    pool.add(make_deploy(2, 0, 2000, 1)).unwrap();
    let removed = pool.prune_expired(5, 0);
    assert!(removed.is_empty());
    assert_eq!(pool.len(), 2);
}

// ── Ancestor scope computation ──

fn make_block_with_real_hash(
    creator: NodeId,
    tag_sig: u8,
    block_number: u64,
    predecessors: HashSet<BlockIdentity>,
    deploys: Vec<SignedDeploy>,
) -> Block {
    let payload = CordialBlockPayload {
        state: BlockState {
            pre_state_hash: vec![tag_sig; 32],
            post_state_hash: vec![tag_sig; 32],
            bonds: vec![Bond { validator: creator.clone(), stake: 100 }],
            block_number,
        },
        deploys: deploys.into_iter().map(|d| ProcessedDeploy {
            deploy: d,
            cost: 100,
            is_failed: false,
        }).collect(),
        rejected_deploys: vec![],
        system_deploys: vec![],
    };
    let content = BlockContent {
        payload: payload.to_bytes(),
        predecessors,
    };
    Block {
        identity: BlockIdentity {
            content_hash: hash_content(&content),
            creator,
            signature: vec![tag_sig],
        },
        content,
    }
}

#[test]
fn compute_deploys_in_scope_empty_for_no_predecessors() {
    let bl = Blocklace::new();
    let scope = compute_deploys_in_scope(&bl, &HashSet::new(), 10, 50);
    assert!(scope.is_empty());
}

#[test]
fn compute_deploys_in_scope_collects_ancestor_deploys() {
    let mut bl = Blocklace::new();
    let v1 = node(1);

    let deploy_a = make_deploy(10, 0, 1000, 1);
    let deploy_b = make_deploy(20, 0, 2000, 1);

    // Genesis with deploy_a
    let g = make_block_with_real_hash(v1.clone(), 1, 0, HashSet::new(), vec![deploy_a.clone()]);
    bl.insert(g.clone()).unwrap();

    // Block 2 with deploy_b, pointing to genesis
    let b2 = make_block_with_real_hash(v1.clone(), 2, 1, [g.identity.clone()].into_iter().collect(), vec![deploy_b.clone()]);
    bl.insert(b2.clone()).unwrap();

    // Compute scope for a new block building on b2
    let predecessors: HashSet<BlockIdentity> = [b2.identity].into_iter().collect();
    let scope = compute_deploys_in_scope(&bl, &predecessors, 10, 50);

    assert_eq!(scope.len(), 2);
    assert!(scope.contains(&deploy_a.signature));
    assert!(scope.contains(&deploy_b.signature));
}

#[test]
fn compute_deploys_in_scope_respects_lifespan_window() {
    let mut bl = Blocklace::new();
    let v1 = node(1);

    let deploy_old = make_deploy(10, 0, 1000, 1);
    let deploy_new = make_deploy(20, 0, 2000, 1);

    // Genesis at block 0 with deploy_old
    let g = make_block_with_real_hash(v1.clone(), 1, 0, HashSet::new(), vec![deploy_old.clone()]);
    bl.insert(g.clone()).unwrap();

    // Block at block_number 100 with deploy_new
    let b2 = make_block_with_real_hash(v1.clone(), 2, 100, [g.identity.clone()].into_iter().collect(), vec![deploy_new.clone()]);
    bl.insert(b2.clone()).unwrap();

    // At current block 110, lifespan 50 => earliest = 60
    // Genesis (block 0) is outside the window — its deploy should NOT be in scope
    let predecessors: HashSet<BlockIdentity> = [b2.identity].into_iter().collect();
    let scope = compute_deploys_in_scope(&bl, &predecessors, 110, 50);

    assert!(scope.contains(&deploy_new.signature));
    assert!(!scope.contains(&deploy_old.signature));
}

// ── End-to-end selection with ancestry dedup ──

#[test]
fn select_excludes_deploys_in_ancestry() {
    let mut bl = Blocklace::new();
    let v1 = node(1);

    let deploy_in_chain = make_deploy(10, 0, 1000, 1);
    let deploy_pending = make_deploy(20, 0, 2000, 1);

    // Build a genesis block that already contains deploy_in_chain
    let g = make_block_with_real_hash(v1.clone(), 1, 0, HashSet::new(), vec![deploy_in_chain.clone()]);
    bl.insert(g.clone()).unwrap();

    // Pool has both deploys
    let mut pool = default_pool();
    pool.add(deploy_in_chain.clone()).unwrap();
    pool.add(deploy_pending.clone()).unwrap();

    // Compute scope from genesis
    let predecessors: HashSet<BlockIdentity> = [g.identity].into_iter().collect();
    let scope = compute_deploys_in_scope(&bl, &predecessors, 1, 50);

    // Select for block #1
    let selected = pool.select_for_block(1, 0, &scope);

    // Only deploy_pending should be selected (deploy_in_chain is in ancestry)
    assert_eq!(selected.deploys.len(), 1);
    assert_eq!(selected.deploys[0].signature, deploy_pending.signature);
}
