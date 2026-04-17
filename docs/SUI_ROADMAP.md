# Roadmap: Moving Closer to Sui Architecture

## Current Status: 31% Sui-like

This document outlines **realistic steps** to become more Sui-like, prioritized by impact vs effort.

---

## 🎯 Three Paths Forward

### Path A: "Sui-Inspired" (Current + Polish)
**Timeline:** 2-3 months  
**Effort:** Medium  
**Outcome:** Better Sui-inspired chain

### Path B: "True Sui-like" (Add Core Features)
**Timeline:** 18-24 months  
**Effort:** Very High  
**Outcome:** Actual Sui competitor

### Path C: "Build on Sui" (Recommended)
**Timeline:** 1-2 months  
**Effort:** Low  
**Outcome:** Production-ready on day 1

---

## 📍 Path A: Polish Current Implementation

### Phase 1: Quick Wins (2 weeks)

#### 1. Add Object References ⭐
**What:** Proper object-to-object references
```rust
pub struct InferenceJob {
    worker_ref: ObjectRef,  // Not just ID
}

pub struct ObjectRef {
    id: ObjectId,
    version: Version,
    digest: [u8; 32],
}
```
**Impact:** More Sui-like semantics  
**Effort:** 3 days

#### 2. Implement Shared vs Owned Objects ⭐⭐
**What:** Distinguish object types
```rust
pub enum ObjectOwnership {
    AddressOwned(Address),
    Shared { initial_version: Version },
    Immutable,
}
```
**Impact:** Enables concurrent access patterns  
**Effort:** 5 days

#### 3. Add Transaction Effects ⭐
**What:** Structured transaction receipts
```rust
pub struct TransactionEffects {
    status: ExecutionStatus,
    gas_used: u64,
    created_objects: Vec<ObjectId>,
    mutated_objects: Vec<(ObjectId, Version)>,
    deleted_objects: Vec<ObjectId>,
    events: Vec<Event>,
}
```
**Impact:** Better debugging and indexing  
**Effort:** 3 days

---

### Phase 2: Architecture Improvements (4 weeks)

#### 4. Basic Gas Model ⭐⭐⭐
**What:** Simple gas mechanism
```rust
pub struct GasConfig {
    base_fee: u64,
    per_byte_fee: u64,
    object_read_fee: u64,
    object_write_fee: u64,
}

pub struct GasBudget {
    max_gas: u64,
    gas_price: u64,
}
```
**Impact:** Spam prevention, economic sustainability  
**Effort:** 2 weeks

#### 5. Transaction Dependency Tracking ⭐⭐⭐
**What:** Build transaction DAG (not execute yet)
```rust
pub struct TransactionDependencies {
    reads: Vec<ObjectId>,
    writes: Vec<ObjectId>,
    dependencies: Vec<TxHash>,
}

// Analyze which transactions can run in parallel
pub fn build_dependency_graph(txs: &[Transaction]) -> DAG<Transaction>
```
**Impact:** Foundation for parallelism  
**Effort:** 2 weeks

#### 6. Object Deletion & Cleanup ⭐
**What:** Allow objects to be deleted
```rust
pub enum Transaction {
    DeleteObject {
        object_id: ObjectId,
        owner: Address,
        version: Version,
    }
}
```
**Impact:** State management, storage efficiency  
**Effort:** 1 week

---

### Phase 3: Developer Experience (2 weeks)

#### 7. Better RPC API ⭐⭐
**What:** Sui-compatible RPC methods
```
POST /sui_getObject
POST /sui_getOwnedObjects
POST /sui_executeTransactionBlock
POST /sui_multiGetObjects
POST /sui_getTransactionBlock
```
**Impact:** Familiar API for developers  
**Effort:** 1 week

#### 8. Object Indexer ⭐⭐
**What:** Queryable object database
```rust
// Postgres schema
CREATE TABLE objects (
    id BYTEA PRIMARY KEY,
    version BIGINT,
    owner TEXT,
    type TEXT,
    data JSONB
);

CREATE INDEX idx_owner ON objects(owner);
CREATE INDEX idx_type ON objects(type);
```
**Impact:** Fast object queries  
**Effort:** 1 week

---

## 📍 Path B: True Sui-like Features

### Phase 1: Move VM Integration (6 months)

#### 9. Integrate Move VM ⭐⭐⭐⭐⭐
**What:** Full Move language support

**Subphases:**
1. **Embed Move VM** (2 months)
   - Include `move-vm-runtime` crate
   - Add Move bytecode verifier
   - Implement VM adapter

2. **Move Stdlib** (1 month)
   - Port Sui framework
   - Implement native functions
   - Add Coin module

3. **Module Publishing** (1 month)
   - Transaction type for module deployment
   - Module upgrade logic
   - Package system

4. **Move Compiler Integration** (1 month)
   - Build system for Move code
   - Source verification
   - Documentation generation

5. **Testing & Debugging** (1 month)
   - Move unit tests
   - Integration tests
   - Debugger support

**Impact:** ⭐⭐⭐⭐⭐ Makes it truly programmable  
**Effort:** 6 months, very complex  
**Team:** 2-3 senior engineers

---

### Phase 2: Parallel Execution (4 months)

#### 10. Parallel Transaction Executor ⭐⭐⭐⭐⭐
**What:** Execute independent transactions concurrently

**Subphases:**
1. **DAG Scheduler** (1.5 months)
   ```rust
   pub struct ParallelScheduler {
       dependency_graph: DAG<TxDigest>,
       execution_queue: WorkQueue,
       worker_pool: ThreadPool,
   }
   ```

2. **Object Locking** (1 month)
   - Read-write lock per object
   - Optimistic locking for reads
   - Conflict detection

3. **Execution Engine** (1 month)
   ```rust
   async fn execute_parallel(
       txs: Vec<Transaction>,
       state: &State
   ) -> Vec<TransactionEffects> {
       // Topological sort by dependencies
       // Execute independent txs concurrently
       // Commit results atomically
   }
   ```

4. **Testing & Benchmarking** (0.5 months)
   - Correctness tests
   - Performance benchmarks
   - Stress testing

**Impact:** ⭐⭐⭐⭐⭐ 10-100x throughput increase  
**Effort:** 4 months  
**Team:** 2 senior engineers

---

### Phase 3: BFT Consensus (6 months)

#### 11. Implement Narwhal/Bullshark ⭐⭐⭐⭐⭐
**What:** Byzantine fault-tolerant consensus

**Subphases:**
1. **Narwhal DAG** (2 months)
   - Mempool DAG structure
   - Certificate creation
   - Broadcasting logic

2. **Bullshark Consensus** (2 months)
   - Leader election
   - Commit rules
   - Finality

3. **Validator Set Management** (1 month)
   - Stake tracking
   - Epoch rotation
   - Validator committee

4. **Network Layer** (1 month)
   - P2P networking
   - Message protocols
   - Byzantine resilience

**Impact:** ⭐⭐⭐⭐⭐ Production-ready decentralization  
**Effort:** 6 months  
**Team:** 3-4 senior engineers

---

### Phase 4: Advanced Features (6 months)

#### 12. Storage Rebates ⭐⭐⭐
**Timeline:** 1 month  
**What:** Refund gas on object deletion

#### 13. Capabilities System ⭐⭐⭐
**Timeline:** 2 months  
**What:** Fine-grained access control

#### 14. Programmable Transaction Blocks ⭐⭐⭐⭐
**Timeline:** 2 months  
**What:** Atomic multi-operation transactions

#### 15. zkLogin / Advanced Auth ⭐⭐
**Timeline:** 1 month  
**What:** OAuth-based authentication

---

## 📍 Path C: Build on Real Sui (Recommended)

### Why This is Best Option:

1. **Leverage Existing Infrastructure**
   - Move VM already works
   - Consensus already proven
   - Parallel execution already optimized

2. **Focus on PoCC Logic**
   - Write Move modules for workers
   - Implement job marketplace
   - Build worker runtime

3. **Production Ready Fast**
   - 1-2 months vs 18-24 months
   - Battle-tested platform
   - Active ecosystem

---

### Implementation Plan (2 months)

#### Week 1-2: Setup
```bash
# Install Sui
cargo install --locked --git https://github.com/MystenLabs/sui.git sui

# Create project
sui move new miraset-pocc
```

#### Week 3-4: Worker Module
```move
module miraset::worker {
    use sui::object::{Self, UID};
    use sui::tx_context::TxContext;
    
    struct WorkerRegistration has key, store {
        id: UID,
        owner: address,
        gpu_model: String,
        vram_gib: u64,
        endpoints: vector<String>,
        stake: Coin<SUI>
    }
    
    public entry fun register(
        gpu_model: String,
        vram_gib: u64,
        stake: Coin<SUI>,
        ctx: &mut TxContext
    ) {
        let worker = WorkerRegistration {
            id: object::new(ctx),
            owner: tx_context::sender(ctx),
            gpu_model,
            vram_gib,
            endpoints: vector::empty(),
            stake
        };
        transfer::transfer(worker, tx_context::sender(ctx));
    }
}
```

#### Week 5-6: Job Module
```move
module miraset::job {
    struct InferenceJob has key {
        id: UID,
        requester: address,
        worker: Option<ID>,  // Reference to Worker
        model: String,
        max_tokens: u64,
        escrow: Coin<SUI>,
        status: u8
    }
    
    public entry fun create_job(
        model: String,
        max_tokens: u64,
        payment: Coin<SUI>,
        ctx: &mut TxContext
    ) { /* ... */ }
}
```

#### Week 7-8: Settlement Module
```move
module miraset::settlement {
    struct Epoch has key {
        id: UID,
        epoch_num: u64,
        worker_stats: Table<ID, WorkerStats>,
        reward_pool: Balance<SUI>
    }
    
    public fun settle_epoch(epoch: &mut Epoch) { /* ... */ }
}
```

---

## 💰 Cost-Benefit Analysis

| Path | Time | Cost | Outcome | Risk |
|------|------|------|---------|------|
| **A: Polish** | 2-3 mo | $100k | Better chain | Low |
| **B: True Sui** | 18-24 mo | $2-3M | Sui competitor | Very High |
| **C: Build on Sui** | 1-2 mo | $50k | Production ready | Low |

---

## 🎯 Recommended Strategy

### Immediate (Now):
1. ✅ Acknowledge current state (31% Sui-like)
2. ✅ Implement Path A Phase 1 (quick wins)
3. ✅ Prototype on real Sui (Path C)

### Short-term (3 months):
1. Complete Path A (polish)
2. Deploy MVP on Sui
3. Compare results

### Decision Point (Month 3):
- **If Sui works well:** Commit to Path C
- **If need custom features:** Start Path B Phase 1 (Move VM)
- **If satisfied:** Stay with Path A

---

## 📊 Feature Priority Matrix

```
High Impact, Low Effort:          High Impact, High Effort:
├─ Object references              ├─ Move VM integration
├─ Transaction effects            ├─ Parallel execution
├─ Basic gas model                ├─ BFT consensus
└─ Shared objects                 └─ PTBs

Low Impact, Low Effort:           Low Impact, High Effort:
├─ Object deletion                ├─ zkLogin
├─ Better RPC                     ├─ Advanced consensus
└─ Indexer                        └─ Storage optimization
```

**Focus on top-left quadrant first!**

---

## 🚦 Success Metrics

### Path A Success:
- [ ] Object references working
- [ ] Gas mechanism preventing spam
- [ ] Transaction effects tracked
- [ ] 1000+ TPS achieved

### Path B Success:
- [ ] Move contracts deployable
- [ ] 10k+ TPS with parallelism
- [ ] BFT consensus with 10+ validators
- [ ] Full Sui compatibility

### Path C Success:
- [ ] PoCC modules deployed on Sui
- [ ] Worker runtime functional
- [ ] Jobs executing and settling
- [ ] Mainnet deployment

---

## 💡 Final Recommendation

**Start with Path A + Prototype Path C in parallel**

**Rationale:**
1. Path A improves current codebase (valuable regardless)
2. Path C proves if Sui works for PoCC (fast validation)
3. If Path C succeeds, saves 18+ months of work
4. If Path C fails, Path A is ready as fallback

**Next Steps:**
1. Implement object references (3 days)
2. Deploy simple Move contract on Sui testnet (1 week)
3. Build worker registration PoC on Sui (2 weeks)
4. Compare and decide

---

*Document Version: 1.0*  
*Last Updated: February 3, 2026*  
*Recommended Path: A + C (Dual Track)*
