/// Transaction executor - integrates Move VM, gas metering, and state management
/// Similar to Sui's transaction execution pipeline

use anyhow::{anyhow, Result};
use miraset_core::{Transaction, Object, ObjectId, Address};
use crate::gas::{GasConfig, GasBudget, GasStatus};
use crate::move_vm::{MoveVMRuntime, MoveVMStateView, FunctionId};
use crate::state::State;
use std::sync::Arc;
use tracing::info;

/// Transaction execution context
pub struct ExecutionContext {
    state: State,
    gas_config: Arc<GasConfig>,
    move_runtime: Arc<MoveVMRuntime>,
}

impl ExecutionContext {
    pub fn new(state: State, gas_config: GasConfig) -> Result<Self> {
        let move_runtime = MoveVMRuntime::new()?;

        Ok(Self {
            state,
            gas_config: Arc::new(gas_config),
            move_runtime: Arc::new(move_runtime),
        })
    }

    /// Execute a transaction with gas metering
    pub fn execute_transaction(
        &self,
        tx: Transaction,
        gas_budget: GasBudget,
    ) -> Result<TransactionEffects> {
        let mut gas_status = GasStatus::new(gas_budget, &self.gas_config);

        // Charge base transaction cost
        gas_status.charge_gas(self.gas_config.base_fee)
            .map_err(|e| anyhow!("Failed to charge base fee: {}", e))?;

        let effects = match tx {
            Transaction::Transfer { from, to, amount, .. } => {
                self.execute_transfer(from, to, amount, &mut gas_status)?
            }
            Transaction::CreateObject { creator, data, .. } => {
                self.execute_create_object(creator, data, &mut gas_status)?
            }
            Transaction::MutateObject { object_id, new_data, owner, .. } => {
                self.execute_mutate_object(object_id, new_data, owner, &mut gas_status)?
            }
            Transaction::TransferObject { object_id, from, to, .. } => {
                self.execute_transfer_object(object_id, from, to, &mut gas_status)?
            }
            Transaction::MoveCall { function, type_args, args, sender, .. } => {
                self.execute_move_call(function, type_args, args, sender, &mut gas_status)?
            }
            Transaction::PublishModule { modules, sender, .. } => {
                self.execute_publish_module(modules, sender, &mut gas_status)?
            }
            _ => {
                return Err(anyhow!("Transaction type not supported in executor"));
            }
        };

        // Finalize gas and deduct from sender
        let gas_cost = gas_status.finalize();
        info!("Transaction executed: gas_used={}, storage={}, rebate={}",
            gas_cost.total_gas_used, gas_cost.storage_cost, gas_cost.storage_rebate);

        Ok(effects)
    }

    /// Execute native token transfer
    fn execute_transfer(
        &self,
        from: Address,
        to: Address,
        amount: u64,
        gas: &mut GasStatus,
    ) -> Result<TransactionEffects> {
        // Charge for computation
        gas.charge_computation(1000).map_err(|e| anyhow!(e))?;

        // Check balance (this is a state read)
        gas.charge_object_read(&self.gas_config).map_err(|e| anyhow!(e))?;
        let from_balance = self.state.get_balance(&from);

        if from_balance < amount {
            return Err(anyhow!("Insufficient balance"));
        }

        // Update balances (state writes)
        gas.charge_object_write(32, &self.gas_config).map_err(|e| anyhow!(e))?;
        gas.charge_object_write(32, &self.gas_config).map_err(|e| anyhow!(e))?;

        // Apply state changes
        self.state.add_balance(&from, -(amount as i64));
        self.state.add_balance(&to, amount as i64);

        Ok(TransactionEffects {
            status: ExecutionStatus::Success,
            gas_used: 0, // Will be set by caller
            created: vec![],
            mutated: vec![], // Balance changes don't mutate objects in object model
            deleted: vec![],
            events: vec![],
        })
    }

    /// Execute object creation
    fn execute_create_object(
        &self,
        creator: Address,
        data: miraset_core::ObjectData,
        gas: &mut GasStatus,
    ) -> Result<TransactionEffects> {
        // Serialize object to estimate size
        let serialized = bincode::serialize(&data)
            .map_err(|e| anyhow!("Failed to serialize object: {}", e))?;

        // Charge for object creation and storage
        gas.charge_object_create(serialized.len(), &self.gas_config).map_err(|e| anyhow!(e))?;
        gas.charge_computation(2000).map_err(|e| anyhow!(e))?;

        // Create object
        let object = Object::new(creator, data);
        let object_id = object.id;

        // Store object
        self.state.create_object(object).map_err(|e| anyhow!(e))?;

        Ok(TransactionEffects {
            status: ExecutionStatus::Success,
            gas_used: 0,
            created: vec![object_id],
            mutated: vec![],
            deleted: vec![],
            events: vec![],
        })
    }

    /// Execute object mutation
    fn execute_mutate_object(
        &self,
        object_id: ObjectId,
        new_data: miraset_core::ObjectData,
        owner: Address,
        gas: &mut GasStatus,
    ) -> Result<TransactionEffects> {
        // Read existing object
        gas.charge_object_read(&self.gas_config).map_err(|e| anyhow!(e))?;
        let mut object = self.state.get_object(&object_id)
            .ok_or_else(|| anyhow!("Object not found"))?;

        // Verify ownership
        if object.owner != owner {
            return Err(anyhow!("Not the object owner"));
        }

        // Charge for mutation
        let serialized = bincode::serialize(&new_data)?;
        gas.charge_object_write(serialized.len(), &self.gas_config).map_err(|e| anyhow!(e))?;
        gas.charge_computation(1500).map_err(|e| anyhow!(e))?;

        // Update object
        object.data = new_data;
        object.version += 1;
        self.state.update_object(object).map_err(|e| anyhow!(e))?;

        Ok(TransactionEffects {
            status: ExecutionStatus::Success,
            gas_used: 0,
            created: vec![],
            mutated: vec![object_id],
            deleted: vec![],
            events: vec![],
        })
    }

    /// Execute object ownership transfer
    fn execute_transfer_object(
        &self,
        object_id: ObjectId,
        from: Address,
        to: Address,
        gas: &mut GasStatus,
    ) -> Result<TransactionEffects> {
        // Read object
        gas.charge_object_read(&self.gas_config).map_err(|e| anyhow!(e))?;
        let mut object = self.state.get_object(&object_id)
            .ok_or_else(|| anyhow!("Object not found"))?;

        // Verify ownership
        if object.owner != from {
            return Err(anyhow!("Not the object owner"));
        }

        // Charge for transfer
        gas.charge_object_write(32, &self.gas_config).map_err(|e| anyhow!(e))?;
        gas.charge_computation(500).map_err(|e| anyhow!(e))?;

        // Transfer ownership
        object.owner = to;
        object.version += 1;
        self.state.update_object(object).map_err(|e| anyhow!(e))?;

        Ok(TransactionEffects {
            status: ExecutionStatus::Success,
            gas_used: 0,
            created: vec![],
            mutated: vec![object_id],
            deleted: vec![],
            events: vec![],
        })
    }

    /// Execute Move function call
    fn execute_move_call(
        &self,
        move_fn: miraset_core::MoveFunction,
        _type_args: Vec<String>,
        args: Vec<Vec<u8>>,
        _sender: Address,
        gas: &mut GasStatus,
    ) -> Result<TransactionEffects> {
        // Convert MoveFunction to FunctionId
        let module_id = crate::move_vm::ModuleId::new(
            move_fn.package,
            move_fn.module.clone(),
        );
        let function = FunctionId {
            module: module_id,
            name: move_fn.function,
        };
        // Create a state view for the Move VM
        let state_view = StateViewAdapter {
            state: self.state.clone(),
            move_runtime: Arc::clone(&self.move_runtime),
        };

        // Create Move VM session
        let mut session = self.move_runtime.new_session(&state_view)?;

        // Parse type arguments (simplified)
        let type_tags = vec![]; // TODO: Parse type_args strings to TypeTags

        // Execute the Move function
        let result = session.execute_function(
            &function,
            type_tags,
            args,
            gas.remaining_gas(),
        )?;

        // Charge gas used by Move VM
        gas.charge_computation(result.gas_used).map_err(|e| anyhow!(e))?;

        if !result.success {
            return Err(anyhow!("Move execution failed: {:?}", result.error));
        }

        // Apply session changes
        let changes = session.finish()?;

        Ok(TransactionEffects {
            status: ExecutionStatus::Success,
            gas_used: 0,
            created: changes.objects_created.iter()
                .map(|_| [0u8; 32]) // TODO: Extract actual IDs
                .collect(),
            mutated: vec![],
            deleted: vec![],
            events: vec![],
        })
    }

    /// Execute Move module publishing
    fn execute_publish_module(
        &self,
        modules: Vec<Vec<u8>>,
        _sender: Address,
        gas: &mut GasStatus,
    ) -> Result<TransactionEffects> {
        let mut published_ids = Vec::new();

        for bytecode in modules {
            // Charge for module size
            gas.charge_object_create(bytecode.len(), &self.gas_config).map_err(|e| anyhow!(e))?;
            gas.charge_computation(5000).map_err(|e| anyhow!(e))?; // Module verification is expensive

            // Publish module
            let module_id = self.move_runtime.publish_module(bytecode)?;
            info!("Published module: {:?}", module_id);
            published_ids.push(module_id);
        }

        Ok(TransactionEffects {
            status: ExecutionStatus::Success,
            gas_used: 0,
            created: vec![], // Modules are stored separately
            mutated: vec![],
            deleted: vec![],
            events: vec![],
        })
    }
}

/// Transaction execution effects (similar to Sui's TransactionEffects)
#[derive(Debug, Clone)]
pub struct TransactionEffects {
    pub status: ExecutionStatus,
    pub gas_used: u64,
    pub created: Vec<ObjectId>,
    pub mutated: Vec<ObjectId>,
    pub deleted: Vec<ObjectId>,
    pub events: Vec<Event>,
}

#[derive(Debug, Clone)]
pub enum ExecutionStatus {
    Success,
    Failure { error: String },
}

#[derive(Debug, Clone)]
pub struct Event {
    pub type_: String,
    pub sender: Address,
    pub data: Vec<u8>,
}

/// Adapter to provide state view for Move VM
struct StateViewAdapter {
    state: State,
    move_runtime: Arc<MoveVMRuntime>,
}

impl MoveVMStateView for StateViewAdapter {
    fn get_object(&self, id: &[u8]) -> Option<Vec<u8>> {
        let mut object_id = [0u8; 32];
        if id.len() >= 32 {
            object_id.copy_from_slice(&id[..32]);
        } else {
            return None;
        }

        self.state.get_object(&object_id)
            .and_then(|obj| bincode::serialize(&obj).ok())
    }

    fn get_module_bytecode(&self, module_id: &crate::move_vm::ModuleId) -> Option<Vec<u8>> {
        self.move_runtime.get_module(module_id)
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_execution_context_creation() {
        let state = State::new();
        let gas_config = GasConfig::default();
        let ctx = ExecutionContext::new(state, gas_config);
        assert!(ctx.is_ok());
    }
}
