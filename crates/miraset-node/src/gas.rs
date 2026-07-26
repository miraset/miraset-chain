use serde::{Deserialize, Serialize};

/// Gas price per unit
pub const GAS_PRICE_UNIT: u64 = 1000;

/// Gas budget limits
pub const MIN_GAS_BUDGET: u64 = 1_000_000;
pub const MAX_GAS_BUDGET: u64 = 50_000_000_000;

/// Base costs for operations
pub const BASE_TX_COST: u64 = 1_000;
pub const OBJECT_READ_COST: u64 = 100;
pub const OBJECT_WRITE_COST: u64 = 1_000;
pub const OBJECT_DELETE_COST: u64 = 500;
pub const OBJECT_CREATE_COST: u64 = 2_000;
pub const EVENT_EMIT_COST: u64 = 200;
pub const PER_BYTE_COST: u64 = 10;

/// Storage costs
pub const STORAGE_PRICE_PER_KB: u64 = 100_000;
pub const STORAGE_REBATE_RATE: f64 = 0.99; // 99% refund on deletion

/// Gas configuration for the chain
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GasConfig {
    /// Base transaction fee
    pub base_fee: u64,

    /// Cost per byte of transaction data
    pub per_byte_fee: u64,

    /// Cost to read an object
    pub object_read_cost: u64,

    /// Cost to write/mutate an object
    pub object_write_cost: u64,

    /// Cost to create a new object
    pub object_create_cost: u64,

    /// Cost to delete an object
    pub object_delete_cost: u64,

    /// Cost to emit an event
    pub event_cost: u64,

    /// Storage price per KB
    pub storage_price_per_kb: u64,

    /// Storage rebate rate (0.0 to 1.0)
    pub storage_rebate_rate: f64,
}

impl Default for GasConfig {
    fn default() -> Self {
        Self {
            base_fee: BASE_TX_COST,
            per_byte_fee: PER_BYTE_COST,
            object_read_cost: OBJECT_READ_COST,
            object_write_cost: OBJECT_WRITE_COST,
            object_create_cost: OBJECT_CREATE_COST,
            object_delete_cost: OBJECT_DELETE_COST,
            event_cost: EVENT_EMIT_COST,
            storage_price_per_kb: STORAGE_PRICE_PER_KB,
            storage_rebate_rate: STORAGE_REBATE_RATE,
        }
    }
}

/// Gas budget for a transaction
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GasBudget {
    /// Maximum gas units willing to pay
    pub max_gas_amount: u64,

    /// Gas price per unit
    pub gas_price: u64,

    /// Total budget (max_gas_amount * gas_price)
    pub total_budget: u64,
}

impl GasBudget {
    pub fn new(max_gas_amount: u64, gas_price: u64) -> Result<Self, GasBudgetError> {
        if max_gas_amount < MIN_GAS_BUDGET {
            return Err(GasBudgetError::BudgetTooLow {
                got: max_gas_amount,
                min: MIN_GAS_BUDGET,
            });
        }

        if max_gas_amount > MAX_GAS_BUDGET {
            return Err(GasBudgetError::BudgetTooHigh {
                got: max_gas_amount,
                max: MAX_GAS_BUDGET,
            });
        }

        let total_budget = max_gas_amount.saturating_mul(gas_price);

        Ok(Self {
            max_gas_amount,
            gas_price,
            total_budget,
        })
    }

    pub fn default_budget() -> Self {
        Self {
            max_gas_amount: 10_000_000,
            gas_price: GAS_PRICE_UNIT,
            total_budget: 10_000_000_u64.saturating_mul(GAS_PRICE_UNIT),
        }
    }
}

/// Gas usage tracking during transaction execution
#[derive(Debug, Clone)]
pub struct GasStatus {
    /// Gas budget
    budget: GasBudget,

    /// Gas consumed so far
    gas_used: u64,

    /// Storage costs
    storage_cost: u64,

    /// Storage rebate (for deleted objects)
    storage_rebate: u64,

    /// Breakdown of gas usage
    breakdown: GasBreakdown,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GasBreakdown {
    pub base_cost: u64,
    pub computation_cost: u64,
    pub storage_cost: u64,
    pub object_reads: u64,
    pub object_writes: u64,
    pub object_creates: u64,
    pub object_deletes: u64,
    pub events_emitted: u64,
}

impl GasStatus {
    pub fn new(budget: GasBudget, config: &GasConfig) -> Self {
        let breakdown = GasBreakdown {
            base_cost: config.base_fee,
            ..Default::default()
        };

        Self {
            budget,
            gas_used: config.base_fee,
            storage_cost: 0,
            storage_rebate: 0,
            breakdown,
        }
    }

    /// Construct a meter with an unlimited budget for devnet/testing paths.
    ///
    /// # Safety
    /// This should only be used in tests or in explicitly metered-off modes.
    pub fn unlimited(config: &GasConfig) -> Self {
        let budget = GasBudget {
            max_gas_amount: u64::MAX,
            gas_price: 1,
            total_budget: u64::MAX,
        };
        Self::new(budget, config)
    }

    /// Charge gas for an operation
    pub fn charge_gas(&mut self, amount: u64) -> Result<(), String> {
        let new_total = self.gas_used.saturating_add(amount);

        if new_total > self.budget.max_gas_amount {
            return Err(format!(
                "Out of gas: needed {}, budget {}, used {}",
                amount, self.budget.max_gas_amount, self.gas_used
            ));
        }

        self.gas_used = new_total;
        Ok(())
    }

    /// Charge for reading an object
    pub fn charge_object_read(&mut self, config: &GasConfig) -> Result<(), String> {
        self.breakdown.object_reads += 1;
        self.charge_gas(config.object_read_cost)
    }

    /// Charge for writing an object
    pub fn charge_object_write(
        &mut self,
        size_bytes: usize,
        config: &GasConfig,
    ) -> Result<(), String> {
        self.breakdown.object_writes += 1;
        let size_cost = (size_bytes as u64 / 1024) * config.per_byte_fee;
        let total = config.object_write_cost + size_cost;
        self.charge_gas(total)
    }

    /// Charge for creating an object
    pub fn charge_object_create(
        &mut self,
        size_bytes: usize,
        config: &GasConfig,
    ) -> Result<(), String> {
        self.breakdown.object_creates += 1;
        let size_cost = (size_bytes as u64 / 1024) * config.storage_price_per_kb;
        let total = config.object_create_cost + size_cost;
        self.storage_cost += size_cost;
        self.charge_gas(total)
    }

    /// Charge for deleting an object (and credit rebate)
    pub fn charge_object_delete(
        &mut self,
        size_bytes: usize,
        config: &GasConfig,
    ) -> Result<(), String> {
        self.breakdown.object_deletes += 1;
        let size_cost = (size_bytes as u64 / 1024) * config.storage_price_per_kb;
        let rebate = (size_cost as f64 * config.storage_rebate_rate) as u64;

        self.storage_rebate += rebate;
        self.charge_gas(config.object_delete_cost)?;

        // Rebate is credited at the end
        Ok(())
    }

    /// Charge for emitting an event
    pub fn charge_event(&mut self, config: &GasConfig) -> Result<(), String> {
        self.breakdown.events_emitted += 1;
        self.charge_gas(config.event_cost)
    }

    /// Charge for computation
    pub fn charge_computation(&mut self, amount: u64) -> Result<(), String> {
        self.breakdown.computation_cost += amount;
        self.charge_gas(amount)
    }

    /// Get remaining gas
    pub fn remaining_gas(&self) -> u64 {
        self.budget.max_gas_amount.saturating_sub(self.gas_used)
    }

    /// Finalize gas and calculate total cost
    pub fn finalize(self) -> GasCost {
        let computation_cost = self.gas_used;
        let storage_cost = self.storage_cost;
        let storage_rebate = self.storage_rebate;

        // Total cost = computation + storage - rebate
        let total_cost = computation_cost
            .saturating_add(storage_cost)
            .saturating_sub(storage_rebate);

        // Convert to native tokens (multiply by gas price)
        let total_cost_tokens = total_cost.saturating_mul(self.budget.gas_price);

        GasCost {
            computation_cost,
            storage_cost,
            storage_rebate,
            total_gas_used: total_cost,
            total_cost_tokens,
            breakdown: self.breakdown,
        }
    }

    /// Summary for logging
    pub fn summary(&self) -> String {
        format!(
            "Gas: {}/{} ({}%), Storage: {}, Rebate: {}",
            self.gas_used,
            self.budget.max_gas_amount,
            (self.gas_used * 100) / self.budget.max_gas_amount,
            self.storage_cost,
            self.storage_rebate
        )
    }
}

/// Final gas cost after transaction execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GasCost {
    /// Gas used for computation
    pub computation_cost: u64,

    /// Gas used for storage
    pub storage_cost: u64,

    /// Storage rebate received
    pub storage_rebate: u64,

    /// Total gas units used (after rebate)
    pub total_gas_used: u64,

    /// Total cost in native tokens
    pub total_cost_tokens: u64,

    /// Detailed breakdown
    pub breakdown: GasBreakdown,
}

impl GasCost {
    pub fn zero() -> Self {
        Self {
            computation_cost: 0,
            storage_cost: 0,
            storage_rebate: 0,
            total_gas_used: 0,
            total_cost_tokens: 0,
            breakdown: GasBreakdown::default(),
        }
    }
}

/// Gas budget validation error.
#[derive(Debug, Clone, thiserror::Error)]
pub enum GasBudgetError {
    #[error("gas budget too low: {got} < {min}")]
    BudgetTooLow { got: u64, min: u64 },
    #[error("gas budget too high: {got} > {max}")]
    BudgetTooHigh { got: u64, max: u64 },
}

/// Estimate the minimum gas units required to execute a transaction.
///
/// This is a conservative upper bound used at validation time to reject
/// transactions that cannot possibly afford execution. The actual gas used is
/// charged during block execution.
pub fn estimate_gas_for_transaction(tx: &miraset_core::Transaction, config: &GasConfig) -> u64 {
    use miraset_core::Transaction;

    let mut gas = config.base_fee;

    // Per-byte cost for the serialized transaction payload.
    if let Ok(bytes) = bincode::serialize(tx) {
        gas = gas.saturating_add((bytes.len() as u64).saturating_mul(config.per_byte_fee));
    }

    match tx {
        Transaction::Transfer { .. } => {
            // Two balance reads + two balance writes + small computation.
            gas = gas
                .saturating_add(config.object_read_cost.saturating_mul(2))
                .saturating_add(config.object_write_cost.saturating_mul(2))
                .saturating_add(1_000);
        }
        Transaction::ChatSend { message, .. } => {
            // Base event emit + per-byte cost of message.
            gas = gas.saturating_add(config.event_cost);
            gas = gas.saturating_add((message.len() as u64).saturating_mul(config.per_byte_fee));
        }
        Transaction::CreateObject { data, .. } => {
            // Object creation is the most expensive common operation.
            if let Ok(size) = bincode::serialize(data).map(|b| b.len()) {
                gas = gas.saturating_add(config.object_create_cost.saturating_add(
                    (size as u64 / 1024).saturating_mul(config.storage_price_per_kb),
                ));
            }
            gas = gas.saturating_add(config.event_cost);
        }
        Transaction::MutateObject { new_data, .. } => {
            if let Ok(size) = bincode::serialize(new_data).map(|b| b.len()) {
                gas = gas.saturating_add(
                    config
                        .object_write_cost
                        .saturating_add((size as u64 / 1024).saturating_mul(config.per_byte_fee)),
                );
            }
            gas = gas.saturating_add(config.object_read_cost);
            gas = gas.saturating_add(config.event_cost);
        }
        Transaction::TransferObject { .. } => {
            gas = gas
                .saturating_add(config.object_read_cost)
                .saturating_add(config.object_write_cost)
                .saturating_add(config.event_cost)
                .saturating_add(500);
        }
        Transaction::RegisterWorker {
            supported_models,
            endpoints,
            ..
        } => {
            // Worker registration creates an object.
            let text_size = supported_models.iter().map(|s| s.len()).sum::<usize>()
                + endpoints.iter().map(|s| s.len()).sum::<usize>();
            gas = gas.saturating_add(config.object_create_cost);
            gas = gas.saturating_add(
                (text_size as u64 / 1024).saturating_mul(config.storage_price_per_kb),
            );
            gas = gas.saturating_add(config.event_cost);
        }
        Transaction::SubmitResourceSnapshot { .. } => {
            gas = gas.saturating_add(config.object_read_cost);
            gas = gas.saturating_add(config.event_cost);
        }
        Transaction::CreateJob { model_id, .. } => {
            // Escrow handling + object creation.
            gas = gas
                .saturating_add(config.object_read_cost)
                .saturating_add(config.object_write_cost.saturating_mul(2));
            gas = gas.saturating_add(config.object_create_cost);
            gas = gas.saturating_add((model_id.len() as u64).saturating_mul(config.per_byte_fee));
            gas = gas.saturating_add(config.event_cost);
            // Auto-assignment read of workers.
            gas = gas.saturating_add(config.object_read_cost.saturating_mul(2));
        }
        Transaction::AssignJob { .. } => {
            gas = gas
                .saturating_add(config.object_read_cost)
                .saturating_add(config.object_write_cost)
                .saturating_add(config.event_cost);
        }
        Transaction::SubmitJobResult { .. } => {
            gas = gas
                .saturating_add(config.object_read_cost)
                .saturating_add(config.object_write_cost)
                .saturating_add(config.event_cost);
        }
        Transaction::AnchorReceipt { .. } => {
            gas = gas.saturating_add(config.event_cost);
        }
        Transaction::ChallengeJob { reason, .. } => {
            gas = gas
                .saturating_add(config.object_read_cost)
                .saturating_add(config.object_write_cost)
                .saturating_add(config.event_cost);
            gas = gas.saturating_add((reason.len() as u64).saturating_mul(config.per_byte_fee));
        }
    }

    gas
}

/// Compute the total native-token cost for a transaction given its gas usage.
pub fn gas_cost_tokens(gas_units: u64, gas_price: u64) -> u64 {
    gas_units.saturating_mul(gas_price)
}

/// Gas coin object for paying fees
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GasCoin {
    pub balance: u64,
}

impl GasCoin {
    pub fn new(balance: u64) -> Self {
        Self { balance }
    }

    pub fn deduct(&mut self, amount: u64) -> Result<(), String> {
        if self.balance < amount {
            return Err(format!("Insufficient gas: {} < {}", self.balance, amount));
        }
        self.balance -= amount;
        Ok(())
    }

    pub fn refund(&mut self, amount: u64) {
        self.balance = self.balance.saturating_add(amount);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn test_gas_budget() {
        let budget = GasBudget::new(10_000_000, GAS_PRICE_UNIT).unwrap();
        assert_eq!(budget.max_gas_amount, 10_000_000);
        assert_eq!(budget.gas_price, GAS_PRICE_UNIT);
    }

    #[test]
    fn test_gas_budget_error_is_typed() {
        let err = GasBudget::new(MIN_GAS_BUDGET - 1, GAS_PRICE_UNIT).unwrap_err();
        assert!(matches!(err, GasBudgetError::BudgetTooLow { .. }));
    }

    #[test]
    fn test_gas_status() {
        let config = GasConfig::default();
        let budget = GasBudget::default_budget();
        let mut status = GasStatus::new(budget, &config);

        // Charge for operations
        status.charge_object_read(&config).unwrap();
        status.charge_object_write(1024, &config).unwrap();
        status.charge_event(&config).unwrap();

        assert!(status.gas_used > config.base_fee);
        assert!(status.remaining_gas() < 10_000_000);
    }

    #[test]
    fn test_out_of_gas() {
        let config = GasConfig::default();
        let budget = GasBudget::new(MIN_GAS_BUDGET, GAS_PRICE_UNIT).unwrap();
        let mut status = GasStatus::new(budget, &config);

        // Try to use more gas than budget
        let result = status.charge_gas(MIN_GAS_BUDGET);
        assert!(result.is_err());
    }

    #[test]
    fn test_storage_rebate() {
        let config = GasConfig::default();
        let budget = GasBudget::default_budget();
        let mut status = GasStatus::new(budget, &config);

        // Create then delete object
        status.charge_object_create(10240, &config).unwrap(); // 10 KB
        status.charge_object_delete(10240, &config).unwrap();

        let cost = status.finalize();
        assert!(cost.storage_rebate > 0);
        assert!(cost.storage_rebate < cost.storage_cost);
    }

    #[test]
    fn test_gas_coin() {
        let mut coin = GasCoin::new(1000);

        coin.deduct(500).unwrap();
        assert_eq!(coin.balance, 500);

        coin.refund(100);
        assert_eq!(coin.balance, 600);

        let result = coin.deduct(1000);
        assert!(result.is_err());
    }
}
