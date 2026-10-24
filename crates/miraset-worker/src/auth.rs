/// Shared-secret authentication for node->worker job dispatch.
///
/// This mirrors `miraset_node::auth::DispatchAuth` without making the worker
/// crate depend on the node crate. Uses BLAKE3 keyed hashing for a compact,
/// deterministic authentication tag over the dispatch payload.
pub struct DispatchAuth;

impl DispatchAuth {
    /// Canonical input bytes for the dispatch authentication tag.
    fn tag_input(
        job_id: &miraset_core::ObjectId,
        worker_id: &miraset_core::ObjectId,
        epoch_id: u64,
        model_id: &str,
        max_tokens: u64,
        prompt: Option<&str>,
    ) -> Vec<u8> {
        let mut input = Vec::new();
        input.extend_from_slice(b"miraset-worker-dispatch-v1");
        input.extend_from_slice(job_id.as_ref());
        input.extend_from_slice(worker_id.as_ref());
        input.extend_from_slice(&epoch_id.to_le_bytes());
        input.extend_from_slice(&(model_id.len() as u64).to_le_bytes());
        input.extend_from_slice(model_id.as_bytes());
        input.extend_from_slice(&max_tokens.to_le_bytes());
        match prompt {
            Some(prompt) => {
                input.push(1);
                input.extend_from_slice(blake3::hash(prompt.as_bytes()).as_bytes());
            }
            None => input.push(0),
        }
        input
    }

    /// Produce a 32-byte authentication tag for a dispatch request.
    pub fn sign_dispatch(
        secret: &[u8; 32],
        job_id: &miraset_core::ObjectId,
        worker_id: &miraset_core::ObjectId,
        epoch_id: u64,
        model_id: &str,
        max_tokens: u64,
        prompt: Option<&str>,
    ) -> [u8; 32] {
        let input = Self::tag_input(job_id, worker_id, epoch_id, model_id, max_tokens, prompt);
        blake3::keyed_hash(secret, &input).into()
    }
}

/// Constant-time equality for byte arrays.
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut acc = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        acc |= x ^ y;
    }
    acc == 0
}
