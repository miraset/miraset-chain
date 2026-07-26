use crate::gas::{GasBudget, GasConfig, GasStatus};
use crate::state::State;
/// Transaction executor for native asset and object operations.
use anyhow::{Result, anyhow};
use miraset_core::{Address, Object, ObjectId, Transaction};
use std::sync::Arc;
use tracing::info;

/// Transaction execution context
pub struct ExecutionContext {
    state: State,
    gas_config: Arc<GasConfig>,
}

impl ExecutionContext {
    pub fn new(state: State, gas_config: GasConfig) -> Result<Self> {
        Ok(Self {
            state,
            gas_config: Arc::new(gas_config),
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
        gas_status
            .charge_gas(self.gas_config.base_fee)
            .map_err(|e| anyhow!("Failed to charge base fee: {}", e))?;

        let effects = match tx {
            Transaction::Transfer {
                from, to, amount, ..
            } => self.execute_transfer(from, to, amount, &mut gas_status)?,
            Transaction::CreateObject { creator, data, .. } => {
                self.execute_create_object(creator, data, &mut gas_status)?
            }
            Transaction::MutateObject {
                object_id,
                new_data,
                owner,
                ..
            } => self.execute_mutate_object(object_id, new_data, owner, &mut gas_status)?,
            Transaction::TransferObject {
                object_id,
                from,
                to,
                ..
            } => self.execute_transfer_object(object_id, from, to, &mut gas_status)?,
            _ => {
                return Err(anyhow!("Transaction type not supported in executor"));
            }
        };

        // Finalize gas and deduct from sender
        let gas_cost = gas_status.finalize();
        info!(
            "Transaction executed: gas_used={}, storage={}, rebate={}",
            gas_cost.total_gas_used, gas_cost.storage_cost, gas_cost.storage_rebate
        );

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
        gas.charge_object_read(&self.gas_config)
            .map_err(|e| anyhow!(e))?;
        let from_balance = self.state.get_balance(&from);

        if from_balance < amount {
            return Err(anyhow!("Insufficient balance"));
        }

        // Update balances (state writes)
        gas.charge_object_write(32, &self.gas_config)
            .map_err(|e| anyhow!(e))?;
        gas.charge_object_write(32, &self.gas_config)
            .map_err(|e| anyhow!(e))?;

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
        let serialized =
            bincode::serialize(&data).map_err(|e| anyhow!("Failed to serialize object: {}", e))?;

        // Charge for object creation and storage
        gas.charge_object_create(serialized.len(), &self.gas_config)
            .map_err(|e| anyhow!(e))?;
        gas.charge_computation(2000).map_err(|e| anyhow!(e))?;

        // Create object
        let object = Object::new(creator, data)?;
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
        gas.charge_object_read(&self.gas_config)
            .map_err(|e| anyhow!(e))?;
        let mut object = self
            .state
            .get_object(&object_id)
            .ok_or_else(|| anyhow!("Object not found"))?;

        // Verify ownership
        if object.owner != owner {
            return Err(anyhow!("Not the object owner"));
        }

        // Charge for mutation
        let serialized = bincode::serialize(&new_data)?;
        gas.charge_object_write(serialized.len(), &self.gas_config)
            .map_err(|e| anyhow!(e))?;
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
        gas.charge_object_read(&self.gas_config)
            .map_err(|e| anyhow!(e))?;
        let mut object = self
            .state
            .get_object(&object_id)
            .ok_or_else(|| anyhow!("Object not found"))?;

        // Verify ownership
        if object.owner != from {
            return Err(anyhow!("Not the object owner"));
        }

        // Charge for transfer
        gas.charge_object_write(32, &self.gas_config)
            .map_err(|e| anyhow!(e))?;
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn test_execution_context_creation() {
        let state = State::new();
        let gas_config = GasConfig::default();
        let ctx = ExecutionContext::new(state, gas_config);
        assert!(ctx.is_ok());
    }
}
