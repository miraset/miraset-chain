use clap::{Parser, Subcommand};
use ed25519_dalek::SigningKey;
use hotstuff_rs::types::signer::ValidatorSigner;
use miraset_chain::consensus::{
    GuardIdentity, GuardedNetwork, GuardedSigner, OpenMode, SafetyHalt, SigningGuard,
};
use miraset_chain::core::types::{ComputeReceipt, Transaction};
use miraset_chain::devnet::{self, DevnetSpec};
use miraset_chain::mempool::Mempool;
use miraset_chain::network::{ChannelNetwork, ChannelNetworkHub, TcpNetwork};
use miraset_chain::node::{
    GenesisConfig, MirasetNode, NodeConfig, genesis_state_commitment, init_genesis,
    init_genesis_with_chain_id,
};
use miraset_chain::process_hardening;
use miraset_chain::rpc::start_rpc_server;
use miraset_chain::signer::process::{
    SIGNER_HALT_EXIT_CODE, SignerExit, SignerOptions, run_signer,
};
use miraset_chain::signer::{RemoteSigner, RemoteSignerExpectation};
use miraset_chain::storage::{MemKVStore, RedbKVStore, read_recovery_metadata};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

type CliResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// Time between a safety halt and process exit, so `/health` can report the reason first.
const SAFETY_HALT_GRACE: Duration = Duration::from_secs(5);
const SAFETY_HALT_POLL: Duration = Duration::from_millis(200);
const SIGNING_GUARD_FILE: &str = "signing-guard.redb";
/// How long a validator waits for its remote signer to come up at startup.
const SIGNER_CONNECT_WINDOW: Duration = Duration::from_secs(10);
const SIGNER_CONNECT_RETRY: Duration = Duration::from_millis(250);
/// Distinct exit status for a consensus safety halt (TODO 8.3.2).
const SAFETY_HALT_EXIT_CODE: i32 = 3;

#[derive(Parser, Debug)]
#[command(name = "miraset-chain")]
#[command(
    about = "MIRASET L1 experimental/devnet blockchain node (HotStuff-rs consensus & PoCC accounting; ADR-0016 Proposed)"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Run the in-memory single-validator demo (default when --genesis is not given)
    #[arg(long, default_value_t = false)]
    dev: bool,

    /// Genesis file (JSON); required to run a persistent validator node
    #[arg(long)]
    genesis: Option<PathBuf>,

    /// Hex-encoded Ed25519 validator secret key file (mode 0600). With --genesis, exactly one
    /// of --key-file or --remote-signer is required
    #[arg(long, conflicts_with = "remote_signer")]
    key_file: Option<PathBuf>,

    /// Unix socket of a `miraset-chain signer` process holding the validator key; the node then
    /// holds no key (key custody D3)
    #[arg(long)]
    remote_signer: Option<PathBuf>,

    /// Hex public key the remote signer must serve (optional pin)
    #[arg(long, requires = "remote_signer")]
    validator_pubkey: Option<String>,

    /// Directory holding the node's Redb database
    #[arg(long, default_value = ".miraset_data")]
    data_dir: PathBuf,

    /// Vote signing guard file (default: <data-dir>/signing-guard.redb). Keep it when wiping
    /// chain data: it is what prevents re-signing views this validator already voted in.
    #[arg(long)]
    signing_guard: Option<PathBuf>,

    /// Create the signing guard even though the chain database already has data. Only safe if
    /// this validator key has never signed on this chain (or the original guard is restored).
    #[arg(long, default_value_t = false)]
    init_signing_guard: bool,

    /// RPC listen address
    #[arg(long, default_value = "127.0.0.1:8545")]
    rpc_addr: SocketAddr,

    /// P2P TCP listen address for HotStuff consensus
    #[arg(long, default_value = "127.0.0.1:9948")]
    p2p_addr: SocketAddr,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Generate a validator Ed25519 keypair; the secret is written to --out (mode 0600)
    Keygen {
        #[arg(long)]
        out: PathBuf,
    },
    /// Print the hex public key (account address) for a key file
    Pubkey {
        #[arg(long)]
        key_file: PathBuf,
    },
    /// Create validator keys, a funded faucet key, and genesis.json (no-op if genesis exists)
    DevnetInit {
        #[arg(long)]
        out: PathBuf,
        #[arg(long, default_value_t = 5)]
        validators: usize,
        #[arg(long, default_value_t = 1337)]
        chain_id: u64,
        /// P2P address template; `{i}` is replaced with the validator index
        #[arg(long, default_value = "node{i}:9948")]
        p2p_addr_template: String,
        #[arg(long, default_value_t = 1_000_000_000)]
        faucet_balance: u64,
        /// Write each key to its own directory: <keys-dir>/node{i}/node{i}.key and
        /// <keys-dir>/faucet/faucet.key (one mount per container)
        #[arg(long)]
        keys_dir: Option<PathBuf>,
    },
    /// Print a signed transfer as JSON, ready to POST to /tx/submit
    SignTransfer {
        #[arg(long)]
        key_file: PathBuf,
        #[arg(long)]
        to: String,
        #[arg(long)]
        amount: u64,
        #[arg(long, default_value_t = 10)]
        fee: u64,
        #[arg(long)]
        nonce: u64,
    },
    /// Run a validator signer holding the key and its slashing-protection guard, serving
    /// gRPC on a Unix socket (only the same uid may connect)
    Signer {
        #[arg(long)]
        genesis: PathBuf,
        #[arg(long)]
        key_file: PathBuf,
        /// Slashing-protection guard file; keep it with the key, never delete it after signing
        #[arg(long)]
        signing_guard: PathBuf,
        #[arg(long)]
        socket: PathBuf,
        /// Create the guard on first start. Only safe if this key has never signed on this chain
        #[arg(long, default_value_t = false)]
        init_signing_guard: bool,
    },
    /// Run the interactive in-memory devnet demo
    Demo,
}

#[tokio::main]
async fn main() -> CliResult {
    let cli = Cli::parse();
    // Before any key is read: no core dumps, no same-user ptrace (key custody D4).
    process_hardening::disable_core_dumps()?;
    match cli.command {
        Some(Commands::Keygen { out }) => keygen(&out),
        Some(Commands::Pubkey { key_file }) => {
            let key = devnet::read_key_file(&key_file)?;
            println!("{}", hex::encode(key.verifying_key().to_bytes()));
            Ok(())
        }
        Some(Commands::DevnetInit {
            out,
            validators,
            chain_id,
            p2p_addr_template,
            faucet_balance,
            keys_dir,
        }) => devnet_init(&DevnetSpec {
            out_dir: &out,
            validators,
            chain_id,
            p2p_addr_template: &p2p_addr_template,
            faucet_balance,
            keys_dir: keys_dir.as_deref(),
        }),
        Some(Commands::SignTransfer {
            key_file,
            to,
            amount,
            fee,
            nonce,
        }) => sign_transfer(&key_file, &to, amount, fee, nonce),
        Some(Commands::Signer {
            genesis,
            key_file,
            signing_guard,
            socket,
            init_signing_guard,
        }) => {
            let options = SignerOptions {
                genesis,
                key_file,
                signing_guard,
                socket,
                init_signing_guard,
            };
            let shutdown = async {
                let _ = shutdown_signal().await;
            };
            match run_signer(options, shutdown).await {
                Ok(()) => Ok(()),
                Err(SignerExit::Error(e)) => Err(e.into()),
                Err(SignerExit::Halted(reason)) => {
                    eprintln!(
                        "signer safety halt: {reason}; exiting with code {SIGNER_HALT_EXIT_CODE}"
                    );
                    std::process::exit(SIGNER_HALT_EXIT_CODE);
                }
            }
        }
        Some(Commands::Demo) => {
            run_devnet_demo().await;
            Ok(())
        }
        None => match (cli.dev, cli.genesis.as_deref()) {
            (false, Some(genesis)) => run_validator(&cli, genesis).await,
            _ => {
                run_devnet_demo().await;
                Ok(())
            }
        },
    }
}

fn keygen(out: &Path) -> CliResult {
    let key = devnet::generate_signing_key();
    devnet::write_key_file(out, &key)?;
    println!(
        "Public key: {}",
        hex::encode(key.verifying_key().to_bytes())
    );
    println!("Secret key written to {} (mode 0600)", out.display());
    Ok(())
}

fn devnet_init(spec: &DevnetSpec<'_>) -> CliResult {
    let created = devnet::init_devnet(spec)?;
    let path = spec.out_dir.join(devnet::GENESIS_FILE);
    if created {
        println!(
            "Created devnet with {} validators at {}",
            spec.validators,
            path.display()
        );
    } else {
        println!(
            "Devnet already initialized at {}; leaving it unchanged",
            path.display()
        );
    }
    Ok(())
}

fn sign_transfer(key_file: &Path, to: &str, amount: u64, fee: u64, nonce: u64) -> CliResult {
    let key = devnet::read_key_file(key_file)?;
    let sender = hex::encode(key.verifying_key().to_bytes());
    let mut tx = Transaction::new(&sender, nonce, to, amount, fee);
    tx.sign_with(&key);
    println!("{}", serde_json::to_string(&tx)?);
    Ok(())
}

/// Run a persistent validator from a genesis file until SIGINT or SIGTERM.
async fn run_validator(cli: &Cli, genesis_path: &Path) -> CliResult {
    let genesis = GenesisConfig::load(genesis_path)?;
    let node_key = load_node_key(cli, genesis.chain_id).await?;
    let vk = node_key.public_key();
    if !genesis.is_validator(&vk) {
        return Err(format!(
            "key {} is not a validator in {}",
            hex::encode(vk.to_bytes()),
            genesis_path.display()
        )
        .into());
    }

    std::fs::create_dir_all(&cli.data_dir)?;
    let safety_halt = SafetyHalt::new();
    safety_halt.install_consensus_panic_hook();
    let kv_store = RedbKVStore::open(cli.data_dir.join("chain.redb"))
        .map_err(|e| format!("cannot open database: {e}"))?
        .with_safety_halt(safety_halt.clone());
    let genesis_hash = genesis_state_commitment(&genesis.initial_accounts());
    let guard = open_signing_guard(cli, &kv_store, genesis.chain_id, &vk, genesis_hash)?;
    init_genesis_with_chain_id(
        kv_store.clone(),
        genesis.chain_id,
        &genesis.validator_powers()?,
        &genesis.initial_accounts(),
    );
    genesis.verify_store_validator_set(&kv_store)?;

    let peers = genesis.peer_addresses(&vk)?;
    let (tcp, runtime) = TcpNetwork::bind_with_hosts(vk, cli.p2p_addr, peers)?;
    // Outermost layer: TcpNetwork delivers self-addressed votes inside `send`.
    let network = GuardedNetwork::new(tcp, guard.clone(), safety_halt.clone());

    // Key custody D1-D3: a local key moves into a GuardedSigner that checks the guard before
    // signing a phase vote; a remote signer holds key and guard in its own process. Either way
    // GuardedNetwork keeps this node's guard as a second layer.
    let signer: Arc<dyn ValidatorSigner> = match node_key {
        NodeKey::Local(signing_key) => GuardedSigner::new(*signing_key, guard)?,
        NodeKey::Remote(remote) => remote,
    };
    let mut config = NodeConfig::devnet_with_signer(signer);
    config.chain_id = genesis.chain_id;
    config.expected_genesis_hash = Some(genesis_hash);
    config.safety_halt = safety_halt.clone();
    let node = Arc::new(MirasetNode::try_start(
        config,
        kv_store,
        network,
        Mempool::new(),
    )?);

    println!("Starting MIRASET experimental/devnet validator (ADR-0016 Proposed):");
    println!("  Chain ID:   {}", genesis.chain_id);
    println!("  Public Key: {}", hex::encode(vk.to_bytes()));
    println!("  Validators: {}", genesis.validators.len());
    println!("  Data Dir:   {}", cli.data_dir.display());
    println!("  P2P Addr:   {}", cli.p2p_addr);
    println!("  RPC Addr:   {}", cli.rpc_addr);

    let mut rpc = tokio::spawn(start_rpc_server(
        Arc::clone(&node),
        genesis.chain_id,
        cli.rpc_addr,
    ));
    let outcome: CliResult = tokio::select! {
        signal = shutdown_signal() => signal.map_err(Into::into),
        reason = wait_for_safety_halt(&safety_halt) => {
            // No graceful shutdown: the replica thread may have panicked, and joining it would
            // panic again. Redb commits are atomic, so exiting here loses nothing durable.
            eprintln!("consensus safety halt: {reason}; exiting with code {SAFETY_HALT_EXIT_CODE}");
            std::process::exit(SAFETY_HALT_EXIT_CODE);
        }
        result = &mut rpc => match result {
            Ok(Ok(())) => Err("RPC server exited unexpectedly".into()),
            Ok(Err(e)) => Err(format!("RPC server failed: {e}").into()),
            Err(e) => Err(format!("RPC task failed: {e}").into()),
        },
    };

    // Stop admission first so the RPC task releases its handle to the node, then stop consensus
    // (joins replica threads; Redb commits are atomic) and finally the transport.
    rpc.abort();
    let _ = rpc.await;
    println!("Shutting down node...");
    drop(node);
    drop(runtime);
    println!("Shutdown complete.");
    outcome
}

/// Where this validator's signatures come from.
enum NodeKey {
    Local(Box<SigningKey>),
    Remote(Arc<RemoteSigner>),
}

impl NodeKey {
    fn public_key(&self) -> ed25519_dalek::VerifyingKey {
        match self {
            NodeKey::Local(key) => key.verifying_key(),
            NodeKey::Remote(remote) => remote.public_key(),
        }
    }
}

async fn load_node_key(cli: &Cli, chain_id: u64) -> CliResult<NodeKey> {
    match (&cli.key_file, &cli.remote_signer) {
        (Some(key_path), None) => Ok(NodeKey::Local(Box::new(devnet::read_key_file(key_path)?))),
        (None, Some(socket)) => {
            let validator = cli
                .validator_pubkey
                .as_deref()
                .map(parse_pubkey)
                .transpose()?;
            let expect = RemoteSignerExpectation {
                chain_id,
                validator,
            };
            let socket = socket.clone();
            // The client blocks on its own runtime; keep it off this async runtime's workers.
            let remote =
                tokio::task::spawn_blocking(move || connect_signer(&socket, &expect)).await??;
            Ok(NodeKey::Remote(Arc::new(remote)))
        }
        _ => Err("with --genesis, pass exactly one of --key-file or --remote-signer".into()),
    }
}

/// Connect to the signer, retrying briefly: an orchestrator may start node and signer together.
/// An identity mismatch is not retried.
fn connect_signer(socket: &Path, expect: &RemoteSignerExpectation) -> Result<RemoteSigner, String> {
    let deadline = std::time::Instant::now() + SIGNER_CONNECT_WINDOW;
    loop {
        match RemoteSigner::connect(socket, expect) {
            Ok(remote) => return Ok(remote),
            Err(e) if e.starts_with("cannot connect") && std::time::Instant::now() < deadline => {
                std::thread::sleep(SIGNER_CONNECT_RETRY);
            }
            Err(e) => return Err(e),
        }
    }
}

fn parse_pubkey(hex_key: &str) -> CliResult<[u8; 32]> {
    hex::decode(hex_key.trim())
        .ok()
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or_else(|| "--validator-pubkey must be 32 hex-encoded bytes".into())
}

/// Open the vote signing guard (TODO 8.3.2). A non-empty chain database requires an existing
/// guard unless `--init-signing-guard` is given, so a lost guard is never silently recreated.
fn open_signing_guard(
    cli: &Cli,
    kv_store: &RedbKVStore,
    chain_id: u64,
    vk: &ed25519_dalek::VerifyingKey,
    genesis_hash: [u8; 32],
) -> CliResult<SigningGuard> {
    let path = cli
        .signing_guard
        .clone()
        .unwrap_or_else(|| cli.data_dir.join(SIGNING_GUARD_FILE));
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let chain_has_data = read_recovery_metadata(kv_store)
        .map_err(|e| format!("cannot read recovery metadata: {e}"))?
        .is_some();
    let mode = OpenMode::for_startup(chain_has_data, cli.init_signing_guard);
    let identity = GuardIdentity {
        chain_id,
        validator: vk.to_bytes(),
        genesis_hash,
    };
    // `to_string` so `main` prints the guard's operator-facing message, not its Debug form.
    let guard = SigningGuard::open(&path, identity, mode).map_err(|e| e.to_string())?;
    let last_view = guard
        .last_vote()
        .map_err(|e| e.to_string())?
        .map(|record| record.view);
    println!(
        "  Signing guard: {} (last signed view: {})",
        path.display(),
        last_view.map_or_else(|| "none".to_string(), |view| view.to_string())
    );
    Ok(guard)
}

/// Resolve once the safety halt fires, after a grace period that lets `/health` report it.
async fn wait_for_safety_halt(halt: &SafetyHalt) -> String {
    while !halt.is_halted() {
        tokio::time::sleep(SAFETY_HALT_POLL).await;
    }
    tokio::time::sleep(SAFETY_HALT_GRACE).await;
    halt.reason().unwrap_or("unknown").to_string()
}

/// Resolve on SIGINT (Ctrl+C) or, on Unix, SIGTERM (`docker stop`).
async fn shutdown_signal() -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut terminate = signal(SignalKind::terminate())?;
        tokio::select! {
            result = tokio::signal::ctrl_c() => result,
            _ = terminate.recv() => Ok(()),
        }
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await
    }
}

async fn run_devnet_demo() {
    println!("=======================================================");
    println!("     MIRASET Chain L1 - HotStuff-rs Consensus Devnet   ");
    println!("=======================================================\n");

    // 1. Generate validator keys
    let validator_key = SigningKey::from_bytes(&[42u8; 32]);
    let vk = validator_key.verifying_key();
    let vk_hex = hex_encode(&vk.to_bytes());
    println!("[1] Initializing Validator Set:");
    println!("    - Validator 0: {} (Power: 100)", vk_hex);

    // 2. Set up Storage and Network
    let kv_store = MemKVStore::new();
    let hub = ChannelNetworkHub::new();
    let network = ChannelNetwork::new(vk, hub);

    // 3. Initialize Genesis State. Alice's account is keyed by her Ed25519 address
    // (not a free-text label) since transactions must be signed by the account they debit.
    let alice_key = SigningKey::from_bytes(&[7u8; 32]);
    let alice_addr = hex::encode(alice_key.verifying_key().to_bytes());
    let coordinator_key = SigningKey::from_bytes(&[8u8; 32]);

    let validators = vec![(vk, 100)];
    let initial_accounts = vec![
        (alice_addr.as_str(), 1_000_000),
        ("bob", 250_000),
        ("worker_4090", 10_000),
    ];

    println!("\n[2] Initializing Genesis Accounts:");
    for (acc, bal) in &initial_accounts {
        println!("    - Account '{}': {} MIRA", acc, bal);
    }
    println!("    (Account '{alice_addr}' is addressed below as 'alice')");
    init_genesis(kv_store.clone(), &validators, &initial_accounts);

    // 4. Start Node
    println!("\n[3] Starting MIRASET Replica Node (Chain ID: 1337)...");
    let mempool = Mempool::new();
    let config = NodeConfig::devnet(validator_key);
    let chain_id = config.chain_id;
    let node = Arc::new(MirasetNode::start(config, kv_store, network, mempool));

    // 5. Start Node RPC API Gateway
    let rpc_addr: SocketAddr = "127.0.0.1:8545".parse().unwrap();
    let rpc_node = Arc::clone(&node);
    tokio::spawn(async move {
        if let Err(e) = start_rpc_server(rpc_node, chain_id, rpc_addr).await {
            eprintln!("RPC server error: {}", e);
        }
    });
    println!("    ✓ Node RPC API listening on http://{}", rpc_addr);
    println!(
        "      Endpoints: /health, /account/:addr, /worker/:id, /task/:id, /tx/submit, /receipt/submit, /block/latest, /events/blocks"
    );

    // Verify initial account queries
    if let Some(alice) = node.get_account(&alice_addr) {
        println!(
            "    ✓ Alice initial balance verified: {} MIRA (nonce: {})",
            alice.balance, alice.nonce
        );
    }

    // 6. Submit sample transaction, signed by Alice's key
    println!("\n[4] Submitting Transactions to Mempool:");
    let mut tx1 = Transaction::new(&alice_addr, 0, "bob", 75_000, 150);
    tx1.sign_with(&alice_key);
    println!("    - Tx: Alice -> Bob (Amount: 75,000, Fee: 150)");
    node.submit_transaction(tx1).expect("Tx submission failed");

    // 7. Submit sample PoCC compute receipt, co-signed by the coordinator
    println!("\n[5] Submitting PoCC Compute Receipt to Mempool:");
    let mut receipt1 = ComputeReceipt::new(
        "inference_task_gpt_oss_1",
        &alice_addr,
        "worker_4090",
        "coordinator_east",
        "sha256:7f83b1657ff1fc53b92dc18148a1d65dfc2d4b1fa3d677284addd200126d9069",
        "zkp:0x892a71f0923058b73a4cf02",
        250,
        600,
        500,
    );
    receipt1.sign_with(&coordinator_key);
    println!(
        "    - Worker: {} completed task '{}' (Reward: 500 MIRA, Client Cost: 600 MIRA)",
        receipt1.worker_id, receipt1.task_id
    );
    node.submit_receipt(receipt1)
        .expect("Receipt submission failed");

    println!("\n[6] Waiting for HotStuff consensus round to propose and commit block...");
    tokio::time::sleep(Duration::from_millis(2000)).await;

    // 8. Check committed state
    println!("\n[7] Querying Node State after Consensus:");
    if let Some(block_hash) = node.highest_committed_block() {
        println!("    ✓ Highest Committed Block Hash: {}", block_hash);
    } else {
        println!("    ℹ Consensus in progress, block proposed.");
    }

    println!("\n[8] Shutting down node gracefully...");
    drop(node);
    println!("✓ Node shutdown complete.");
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}
