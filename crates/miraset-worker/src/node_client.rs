/// Client for interacting with Miraset node RPC
use anyhow::Result;
use miraset_core::{Address, KeyPair, ObjectId, Transaction};
use serde::{Deserialize, Serialize};

#[derive(Clone)]
pub struct NodeClient {
    base_url: String,
    keypair: KeyPair,
    client: reqwest::Client,
}

#[derive(Debug, Serialize, Deserialize)]
struct BalanceResponse {
    balance: u64,
}

#[derive(Debug, Serialize, Deserialize)]
struct NonceResponse {
    nonce: u64,
}

impl NodeClient {
    pub fn new(base_url: String, keypair: KeyPair) -> Self {
        Self {
            base_url,
            keypair,
            client: reqwest::Client::new(),
        }
    }

    /// Get current nonce for this worker's address
    pub async fn get_nonce(&self) -> Result<u64> {
        let addr = self.keypair.address();
        let url = format!("{}/nonce/{}", self.base_url, addr.to_hex());

        let response = self.client.get(&url).send().await?;
        let nonce: u64 = response.json().await?;

        Ok(nonce)
    }

    /// Submit a signed transaction to the node
    pub async fn submit_transaction(&self, tx: Transaction) -> Result<()> {
        let url = format!("{}/tx/submit", self.base_url);

        let response = self.client
            .post(&url)
            .json(&tx)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            anyhow::bail!("Failed to submit transaction: {}", error_text);
        }

        Ok(())
    }

    /// Register this worker on-chain
    pub async fn register_worker(
        &self,
        endpoints: Vec<String>,
        gpu_model: String,
        vram_total_gib: u32,
        supported_models: Vec<String>,
        stake_bond: u64,
    ) -> Result<ObjectId> {
        let nonce = self.get_nonce().await?;
        let owner = self.keypair.address();
        let pubkey = owner; // For simplicity, using same key

        // Create transaction payload for signing
        let mut tx_data = Vec::new();
        tx_data.extend_from_slice(owner.as_bytes());
        tx_data.extend_from_slice(pubkey.as_bytes());
        tx_data.extend_from_slice(&bincode::serialize(&endpoints)?);
        tx_data.extend_from_slice(&bincode::serialize(&gpu_model)?);
        tx_data.extend_from_slice(&vram_total_gib.to_le_bytes());
        tx_data.extend_from_slice(&bincode::serialize(&supported_models)?);
        tx_data.extend_from_slice(&stake_bond.to_le_bytes());
        tx_data.extend_from_slice(&nonce.to_le_bytes());

        let signature = self.keypair.sign(&tx_data);

        let tx = Transaction::RegisterWorker {
            owner,
            pubkey,
            endpoints,
            gpu_model,
            vram_total_gib,
            supported_models,
            stake_bond,
            nonce,
            signature,
        };

        self.submit_transaction(tx).await?;

        // Compute worker_id (matches node logic)
        let worker_id_data = bincode::serialize(&(owner, pubkey))?;
        let worker_id = blake3::hash(&worker_id_data).into();

        Ok(worker_id)
    }

    /// Submit job result to chain
    pub async fn submit_job_result(
        &self,
        job_id: ObjectId,
        worker_id: ObjectId,
        output_tokens: u64,
        receipt_hash: [u8; 32],
    ) -> Result<()> {
        let nonce = self.get_nonce().await?;
        let worker = self.keypair.address();

        // Create transaction payload for signing
        let mut tx_data = Vec::new();
        tx_data.extend_from_slice(&job_id);
        tx_data.extend_from_slice(&worker_id);
        tx_data.extend_from_slice(&output_tokens.to_le_bytes());
        tx_data.extend_from_slice(&receipt_hash);
        tx_data.extend_from_slice(worker.as_bytes());
        tx_data.extend_from_slice(&nonce.to_le_bytes());

        let signature = self.keypair.sign(&tx_data);

        let tx = Transaction::SubmitJobResult {
            job_id,
            worker_id,
            output_tokens,
            receipt_hash,
            worker,
            nonce,
            signature,
        };

        self.submit_transaction(tx).await?;

        tracing::info!(
            "Submitted job result: job={:?}, tokens={}",
            job_id,
            output_tokens
        );

        Ok(())
    }

    /// Anchor receipt hash on-chain
    pub async fn anchor_receipt(
        &self,
        job_id: ObjectId,
        receipt_hash: [u8; 32],
    ) -> Result<()> {
        let nonce = self.get_nonce().await?;
        let submitter = self.keypair.address();

        let mut tx_data = Vec::new();
        tx_data.extend_from_slice(&job_id);
        tx_data.extend_from_slice(&receipt_hash);
        tx_data.extend_from_slice(submitter.as_bytes());
        tx_data.extend_from_slice(&nonce.to_le_bytes());

        let signature = self.keypair.sign(&tx_data);

        let tx = Transaction::AnchorReceipt {
            job_id,
            receipt_hash,
            submitter,
            nonce,
            signature,
        };

        self.submit_transaction(tx).await?;

        tracing::info!("Anchored receipt: job={:?}", job_id);

        Ok(())
    }
}
