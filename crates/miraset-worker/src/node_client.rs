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

        // Build TX with zero signature, serialize, sign, then fill signature
        let mut tx = Transaction::RegisterWorker {
            owner,
            pubkey,
            endpoints: endpoints.clone(),
            gpu_model: gpu_model.clone(),
            vram_total_gib,
            supported_models: supported_models.clone(),
            stake_bond,
            nonce,
            signature: [0; 64],
        };

        let msg = bincode::serialize(&tx)?;
        let signature = self.keypair.sign(&msg);

        if let Transaction::RegisterWorker { signature: ref mut sig, .. } = tx {
            *sig = signature;
        }

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

        let mut tx = Transaction::SubmitJobResult {
            job_id,
            worker_id,
            output_tokens,
            receipt_hash,
            worker,
            nonce,
            signature: [0; 64],
        };

        let msg = bincode::serialize(&tx)?;
        let signature = self.keypair.sign(&msg);

        if let Transaction::SubmitJobResult { signature: ref mut sig, .. } = tx {
            *sig = signature;
        }

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

        let mut tx = Transaction::AnchorReceipt {
            job_id,
            receipt_hash,
            submitter,
            nonce,
            signature: [0; 64],
        };

        let msg = bincode::serialize(&tx)?;
        let signature = self.keypair.sign(&msg);

        if let Transaction::AnchorReceipt { signature: ref mut sig, .. } = tx {
            *sig = signature;
        }

        self.submit_transaction(tx).await?;

        tracing::info!("Anchored receipt: job={:?}", job_id);

        Ok(())
    }

    /// Submit resource snapshot (heartbeat with VRAM data)
    pub async fn submit_resource_snapshot(
        &self,
        worker_id: ObjectId,
        epoch_id: u64,
        vram_avail_gib: u32,
    ) -> Result<()> {
        let nonce = self.get_nonce().await?;
        let owner = self.keypair.address();

        let mut tx = Transaction::SubmitResourceSnapshot {
            worker_id,
            epoch_id,
            vram_avail_gib,
            owner,
            nonce,
            signature: [0; 64],
        };

        let msg = bincode::serialize(&tx)?;
        let signature = self.keypair.sign(&msg);

        if let Transaction::SubmitResourceSnapshot { signature: ref mut sig, .. } = tx {
            *sig = signature;
        }

        self.submit_transaction(tx).await?;

        tracing::debug!(
            "Submitted resource snapshot: worker={}, vram={}GiB",
            hex::encode(worker_id),
            vram_avail_gib,
        );

        Ok(())
    }

    /// Get current epoch info from node
    pub async fn get_epoch(&self) -> Result<serde_json::Value> {
        let url = format!("{}/epoch", self.base_url);
        let response = self.client.get(&url).send().await?;
        Ok(response.json().await?)
    }

    /// Ping node health
    pub async fn ping(&self) -> Result<bool> {
        let url = format!("{}/ping", self.base_url);
        match self.client.get(&url).send().await {
            Ok(resp) => Ok(resp.status().is_success()),
            Err(_) => Ok(false),
        }
    }
}
