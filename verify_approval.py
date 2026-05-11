import collections

class BlockIdentity:
    def __init__(self, creator, content_hash):
        self.creator = creator
        self.content_hash = content_hash
    def __eq__(self, other):
        return self.creator == other.creator and self.content_hash == other.content_hash
    def __hash__(self):
        return hash((self.creator, self.content_hash))

class BlockContent:
    def __init__(self, predecessors):
        self.predecessors = predecessors

class Block:
    def __init__(self, creator, content_hash, predecessors):
        self.identity = BlockIdentity(creator, content_hash)
        self.content = BlockContent(predecessors)

class Blocklace:
    def __init__(self):
        self.blocks = {}
    def insert(self, block):
        self.blocks[block.identity] = block
    def get(self, id):
        return self.blocks.get(id)
    def observe(self, from_id):
        visited = set()
        queue = [from_id]
        visited.add(from_id)
        while queue:
            current_id = queue.pop(0)
            block = self.get(current_id)
            if block:
                for pred in block.content.predecessors:
                    if pred not in visited:
                        visited.add(pred)
                        queue.append(pred)
        return visited

def get_depth(blocklace, block_id):
    block = blocklace.get(block_id)
    if not block or not block.content.predecessors:
        return 0
    return 1 + max(get_depth(blocklace, p) for p in block.content.predecessors)

def approves_binary(blocklace, approver_id, candidate_id):
    candidate = blocklace.get(candidate_id)
    if not candidate: return False
    
    approver = blocklace.get(approver_id)
    if not approver: return False
    
    # 1. Observation
    observed = blocklace.observe(approver_id)
    if candidate_id not in observed:
        return False
        
    # 2. Equivocation
    creator = candidate.identity.creator
    round = get_depth(blocklace, candidate_id)
    
    blocks_at_round = 0
    for obs_id in observed:
        if obs_id.creator == creator and get_depth(blocklace, obs_id) == round:
            blocks_at_round += 1
            
    return blocks_at_round < 2

def approves_weighted(blocklace, bonds, approver_id, candidate_id, threshold_num, threshold_den):
    if not approves_binary(blocklace, approver_id, candidate_id):
        return False
        
    observed = blocklace.observe(approver_id)
    support_weight = 0
    seen_validators = set()
    
    for obs_id in observed:
        if obs_id.creator in seen_validators:
            continue
        
        if approves_binary(blocklace, obs_id, candidate_id):
            weight = bonds.get(obs_id.creator, 0)
            support_weight += weight
            seen_validators.add(obs_id.creator)
            
    total_weight = sum(bonds.values())
    if total_weight == 0: return False
    
    return support_weight * threshold_den > total_weight * threshold_num

# --- TESTS ---

def run_tests():
    bl = Blocklace()
    bonds = {"alice": 100, "bob": 0}
    
    aid1 = BlockIdentity("alice", "h1")
    bl.insert(Block("alice", "h1", []))
    
    # Bob creates two blocks observing alice
    bid1 = BlockIdentity("bob", "hb1")
    bl.insert(Block("bob", "hb1", [aid1]))
    
    bid2 = BlockIdentity("bob", "hb2")
    bl.insert(Block("bob", "hb2", [bid1]))
    
    # Charlie observes both
    cid1 = BlockIdentity("charlie", "hc1")
    bl.insert(Block("charlie", "hc1", [bid2]))
    
    # 1. Test double counting: bob has 2 blocks, but weight is 0. Alice has 1 block, weight 100.
    # Total = 100. Support = 100.
    assert approves_weighted(bl, bonds, cid1, aid1, 1, 2) == True
    print("Test 1 (No Double Counting): PASSED")
    
    # 2. Test equivocation
    aid2 = BlockIdentity("alice", "h2")
    bl.insert(Block("alice", "h2", [])) # Alice equivocates in round 0
    
    # David observes both alice blocks
    did1 = BlockIdentity("david", "hd1")
    bl.insert(Block("david", "hd1", [aid1, aid2]))
    
    assert approves_binary(bl, did1, aid1) == False
    print("Test 2 (Equivocation Rejection): PASSED")

    # 3. Test observation
    eid1 = BlockIdentity("eve", "he1")
    bl.insert(Block("eve", "he1", []))
    assert approves_binary(bl, cid1, eid1) == False
    print("Test 3 (Observation Required): PASSED")

if __name__ == "__main__":
    run_tests()
