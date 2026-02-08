# Move VM Integration Guide

## Overview

This guide explains how to integrate the Move VM into Miraset Chain. The integration is currently in **placeholder stage** and requires significant work to become fully functional.

---

## Current Status

### ✅ What's Done
- [x] Gas system infrastructure (complete)
- [x] Gas metering and tracking
- [x] Storage rebates
- [x] Gas budget and cost calculation
- [x] Move VM module structure (placeholder)
- [x] Dependencies added to Cargo.toml

### ⚠️ What's Partial
- [ ] Move VM runtime integration (50% - structure only)
- [ ] Module verification (0%)
- [ ] Function execution (0%)
- [ ] Type system integration (0%)

### ❌ What's Missing
- [ ] Move bytecode verifier integration
- [ ] Move standard library
- [ ] Native function implementations
- [ ] Move-to-Rust adapter layer
- [ ] Module storage and caching
- [ ] Move compiler integration

---

## Why Move VM Integration is Complex

### 1. Move VM is Tightly Coupled with Sui
The Move VM from Sui includes:
- Sui-specific native functions
- Sui object model assumptions
- Sui type system extensions
- Sui standard library dependencies

### 2. Large Dependency Tree
```
move-vm-runtime
├── move-core-types
├── move-binary-format
├── move-bytecode-verifier
├── move-vm-types
├── sui-types
├── sui-framework
└── ... many more
```

### 3. Requires Custom Adapters
- Native function implementations
- Storage interface adapters
- Gas metering integration
- Event system mapping

---

## Architecture Overview

```
┌─────────────────────────────────────────────────┐
│          Miraset Transaction Layer              │
│  (Rust transactions + Move smart contracts)     │
└─────────────────┬───────────────────────────────┘
                  │
                  ▼
┌─────────────────────────────────────────────────┐
│            Move VM Adapter Layer                │
│  - Type conversion (Rust ↔ Move)                │
│  - Native function implementations              │
│  - Gas metering hooks                           │
└─────────────────┬───────────────────────────────┘
                  │
                  ▼
┌─────────────────────────────────────────────────┐
│            Move VM Runtime                      │
│  - Bytecode interpreter                         │
│  - Type safety verification                     │
│  - Reference safety                             │
└─────────────────┬───────────────────────────────┘
                  │
                  ▼
┌─────────────────────────────────────────────────┐
│            State Storage                        │
│  - Object storage                               │
│  - Module storage                               │
│  - Resource storage                             │
└─────────────────────────────────────────────────┘
```

---

## Implementation Plan

### Phase 1: Dependencies (1 week)

#### Step 1.1: Uncomment Move Dependencies
In `crates/miraset-node/Cargo.toml`:
```toml
# Uncomment these:
move-vm-runtime.workspace = true
move-core-types.workspace = true
move-binary-format.workspace = true
move-bytecode-verifier.workspace = true
move-vm-types.workspace = true
```

#### Step 1.2: Build and Resolve Conflicts
```bash
cargo build
# Fix any compilation errors
```

---

### Phase 2: Module Storage (1 week)

#### Step 2.1: Add Module Storage to State
```rust
// In state.rs
struct StateInner {
    // ...existing fields...
    
    // Move modules
    modules: HashMap<ModuleId, CompiledModule>,
}
```

#### Step 2.2: Implement Module Operations
```rust
impl State {
    pub fn publish_module(&self, bytecode: Vec<u8>) -> Result<ModuleId> {
        // 1. Deserialize bytecode
        let module = CompiledModule::deserialize(&bytecode)?;
        
        // 2. Verify module
        move_bytecode_verifier::verify_module(&module)?;
        
        // 3. Check dependencies
        for dep in module.immediate_dependencies() {
            if !self.has_module(&dep) {
                return Err("Missing dependency");
            }
        }
        
        // 4. Store module
        let module_id = ModuleId::from(&module);
        self.inner.write().modules.insert(module_id.clone(), module);
        
        Ok(module_id)
    }
}
```

---

### Phase 3: Native Functions (2 weeks)

#### Step 3.1: Define Native Function Table
```rust
use move_vm_runtime::native_functions::{NativeFunction, NativeFunctionTable};

fn miraset_natives() -> NativeFunctionTable {
    let mut table = NativeFunctionTable::new();
    
    // Object operations
    table.add(
        "miraset",
        "object",
        "new",
        native_object_new,
    );
    
    // Event emission
    table.add(
        "miraset",
        "event",
        "emit",
        native_event_emit,
    );
    
    table
}

fn native_object_new(
    context: &mut NativeContext,
    args: Vec<Value>
) -> PartialVMResult<NativeResult> {
    // Implementation
}
```

#### Step 3.2: Implement Key Natives
Essential native functions needed:
- `object::new()` - Create object
- `object::uid_to_inner()` - Get object ID
- `transfer::transfer()` - Transfer ownership
- `tx_context::sender()` - Get transaction sender
- `event::emit()` - Emit event

---

### Phase 4: VM Session (2 weeks)

#### Step 4.1: Initialize Move VM
```rust
use move_vm_runtime::move_vm::MoveVM;

pub struct MoveVMSession {
    vm: MoveVM,
    session: Session<'_, '_>,
}

impl MoveVMSession {
    pub fn new(state: &mut State) -> Result<Self> {
        let natives = miraset_natives();
        let vm = MoveVM::new(natives)?;
        
        // Create session with state as storage
        let session = vm.new_session(&state);
        
        Ok(Self { vm, session })
    }
}
```

#### Step 4.2: Execute Function
```rust
impl MoveVMSession {
    pub fn execute_function(
        &mut self,
        module: &ModuleId,
        function: &str,
        type_args: Vec<TypeTag>,
        args: Vec<Vec<u8>>,
        gas_budget: u64,
    ) -> Result<Vec<Vec<u8>>> {
        // 1. Load function
        let func = self.session.load_function(
            &module,
            &Identifier::new(function)?,
            &type_args,
        )?;
        
        // 2. Execute with gas
        let result = self.session.execute_function(
            func,
            args,
            gas_budget,
        )?;
        
        Ok(result)
    }
}
```

---

### Phase 5: Integration with Transactions (1 week)

#### Step 5.1: Add Move Transaction Type
```rust
// In types.rs
pub enum Transaction {
    // ...existing types...
    
    MoveCall {
        sender: Address,
        module: ModuleId,
        function: String,
        type_args: Vec<String>,
        args: Vec<Vec<u8>>,
        gas_budget: GasBudget,
        nonce: u64,
        signature: [u8; 64],
    },
    
    PublishModule {
        sender: Address,
        bytecode: Vec<u8>,
        gas_budget: GasBudget,
        nonce: u64,
        signature: [u8; 64],
    },
}
```

#### Step 5.2: Execute in State
```rust
impl State {
    fn execute_move_call(&mut self, tx: &MoveCallTransaction) -> Result<()> {
        // 1. Create VM session
        let mut session = MoveVMSession::new(self)?;
        
        // 2. Execute function
        let results = session.execute_function(
            &tx.module,
            &tx.function,
            tx.type_args.clone(),
            tx.args.clone(),
            tx.gas_budget.max_gas_amount,
        )?;
        
        // 3. Apply changes to state
        session.commit(self)?;
        
        Ok(())
    }
}
```

---

## Estimated Timeline

| Phase | Task | Time | Dependencies |
|-------|------|------|--------------|
| 1 | Dependencies | 1 week | None |
| 2 | Module Storage | 1 week | Phase 1 |
| 3 | Native Functions | 2 weeks | Phase 2 |
| 4 | VM Session | 2 weeks | Phase 3 |
| 5 | Transaction Integration | 1 week | Phase 4 |
| 6 | Testing | 2 weeks | Phase 5 |
| 7 | Documentation | 1 week | Phase 6 |

**Total: ~10 weeks (2.5 months)**

---

## Current Workaround

### Option 1: Use Placeholder Move VM
The current implementation provides:
- Structure for Move integration
- Gas system (fully functional)
- Module and function identifiers
- Type system placeholders

This allows development to continue while Move VM integration is in progress.

### Option 2: Build on Sui
As recommended in `SUI_ROADMAP.md`, the fastest path is:
```bash
# 1. Install Sui
cargo install --locked --git https://github.com/MystenLabs/sui.git sui

# 2. Write Move modules
sui move new miraset-pocc

# 3. Deploy to Sui
sui client publish
```

**Time: 1-2 months vs 6+ months for custom integration**

---

## Testing Strategy

### Unit Tests
```rust
#[test]
fn test_module_publish() {
    let state = State::new();
    let bytecode = compile_move_module("
        module test::hello {
            public fun world(): u64 { 42 }
        }
    ");
    
    let module_id = state.publish_module(bytecode).unwrap();
    assert!(state.has_module(&module_id));
}

#[test]
fn test_function_execution() {
    let state = State::new();
    // Publish module first
    // ...
    
    let result = state.execute_function(
        &module_id,
        "world",
        vec![],
        vec![],
    ).unwrap();
    
    assert_eq!(decode_u64(&result[0]), 42);
}
```

### Integration Tests
```rust
#[test]
fn test_worker_registration_move() {
    // Test worker registration via Move contract
    let move_code = r#"
        module miraset::worker {
            public entry fun register(
                gpu_model: vector<u8>,
                vram_gib: u64,
                ctx: &mut TxContext
            ) {
                // Implementation
            }
        }
    "#;
    
    // Test that Move registration creates same result as Rust
}
```

---

## Resources

### Documentation
- [Move Book](https://move-language.github.io/move/)
- [Sui Move](https://docs.sui.io/concepts/sui-move-concepts)
- [Move VM Source](https://github.com/MystenLabs/sui/tree/main/external-crates/move)

### Examples
- `sui/crates/sui-framework/` - Sui's Move framework
- `sui/crates/sui-adapter/` - Sui's Move VM adapter
- `sui/crates/sui-types/` - Type definitions

### Community
- [Sui Discord](https://discord.gg/sui)
- [Move Forum](https://forum.sui.io/)

---

## Troubleshooting

### Issue: Move Dependencies Won't Compile
**Solution:** Use exact Sui revision
```toml
move-vm-runtime = { git = "https://github.com/MystenLabs/sui", rev = "mainnet-v1.21.0" }
```

### Issue: Type Conflicts with Sui Types
**Solution:** Create adapter layer
```rust
// Convert between Miraset types and Move types
impl From<Address> for MoveAddress {
    fn from(addr: Address) -> Self {
        MoveAddress::new(addr.as_bytes())
    }
}
```

### Issue: Native Functions Not Found
**Solution:** Register all required natives
```rust
let mut natives = NativeFunctionTable::new();
// Register ALL natives your modules use
natives.add("std", "vector", "empty", native_vector_empty);
// ... etc
```

---

## Next Steps

1. ✅ Gas system - COMPLETE
2. ✅ Move VM structure - COMPLETE (placeholder)
3. ⚠️ Uncomment Move dependencies - DO THIS FIRST
4. ⚠️ Implement module storage
5. ⚠️ Implement native functions
6. ⚠️ Create VM session
7. ⚠️ Add Move transactions
8. ⚠️ Test end-to-end

---

## Conclusion

Move VM integration is a **large but achievable** task. The infrastructure (gas, objects, events) is ready. The main work is:

1. **Wiring** Move VM to storage (2 weeks)
2. **Native functions** for Miraset features (2 weeks)
3. **Testing** and debugging (2 weeks)

**Alternative:** Deploy on Sui instead and save 6+ months.

---

*Document Version: 1.0*
*Last Updated: February 3, 2026*
*Status: Gas System Complete, Move VM Placeholder*
