use clap::{Parser, Subcommand};
use ed25519_dalek::SigningKey;
use hotstuff_rs::types::signer::ValidatorSigner;
use miraset_chain::consensus::{
    GuardIdentity, GuardedNetwork, GuardedSigner, OpenMode, SafetyHalt, SigningGuard,
};
use miraset_chain::core::types::{
    AllowanceAction, ComputeReceipt, MIN_WORKER_BOND, ProtocolParams, Transaction, WorkerBondAction,
};
use miraset_chain::devnet::{self, DevnetSpec};
use miraset_chain::mempool::Mempool;
use miraset_chain::network::{ChannelNetwork, ChannelNetworkHub, TcpNetwork, TransportIdentity};
use miraset_chain::node::{GenesisConfig, MirasetNode, NodeConfig, init_genesis_with_params};
use miraset_chain::observability::logging::SAFETY_LOG_TARGET;
use miraset_chain::observability::{LogFormat, StorageFiles, VoteGuardLayer, init_logging};
use miraset_chain::process_hardening;
use miraset_chain::rpc::{self, ApiKeys, RateLimit, RpcAccess, start_rpc_server};
use miraset_chain::signer::process::{
    SIGNER_HALT_EXIT_CODE, SignerExit, SignerOptions, run_signer,
};
use miraset_chain::signer::{RemoteSigner, RemoteSignerExpectation};
use miraset_chain::storage::maintenance::{self, ExpectedChain, MaintenanceError};
use miraset_chain::storage::{MemKVStore, RedbKVStore, read_recovery_metadata};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

type CliResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// Time between a safety halt and process exit, so `/v1/health` can report the reason first.
const SAFETY_HALT_GRACE: Duration = Duration::from_secs(5);
const SAFETY_HALT_POLL: Duration = Duration::from_millis(200);
const SIGNING_GUARD_FILE: &str = "signing-guard.redb";
const CHAIN_DB_FILE: &str = "chain.redb";
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
#[command(arg_required_else_help = true)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Diagnostic log format on stderr (`RUST_LOG` sets the filter, default `info`)
    #[arg(long, value_enum, default_value_t = LogFormat::Text, global = true)]
    log_format: LogFormat,
}

/// `start`: run a persistent validator (TODO 8.4.5: the only way to start one).
#[derive(clap::Args, Debug)]
struct StartArgs {
    /// Genesis file (JSON)
    #[arg(long)]
    genesis: PathBuf,

    /// Hex-encoded Ed25519 validator secret key file (mode 0600). Exactly one of --key-file or
    /// --remote-signer is required
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

    /// `/readyz` reports not ready when no block was committed within this many seconds
    #[arg(long, default_value_t = 30, value_parser = clap::value_parser!(u64).range(1..=86_400))]
    ready_max_commit_age_secs: u64,

    /// File of RPC API keys (`<name> <sha256-hex>` per line, see `rpc-key`; must not be group-
    /// or world-writable; read at start, so revoking a key needs a restart). When set, POST
    /// /v1/transactions and /v1/receipts require `Authorization: Bearer <token>`; without it
    /// submissions are open (devnet only). The RPC is plain HTTP: put TLS in front of it
    /// whenever tokens cross a network
    #[arg(long)]
    rpc_api_keys: Option<PathBuf>,

    /// Submissions per second each client may sustain: per API key name, or per client IP (IPv6
    /// /64) without keys. Behind a reverse proxy all clients share the proxy's IP, so use API
    /// keys there
    #[arg(long, default_value_t = 50.0, value_parser = parse_positive_rate)]
    rpc_submit_rate: f64,

    /// Submissions each client may send at once before the rate applies
    #[arg(long, default_value_t = 100, value_parser = clap::value_parser!(u32).range(1..=1_000_000))]
    rpc_submit_burst: u32,

    /// Strict block-rate limit in milliseconds: no block (empty or not) sooner than this after
    /// its parent; 0 = off (only the 1 s empty-block heartbeat). Capped at half the view time.
    #[arg(long, default_value_t = 0)]
    min_block_interval_ms: u64,

    /// On SIGTERM/SIGINT, refuse admission and report not ready for this long before stopping,
    /// so load balancers and clients move away first
    #[arg(long, default_value_t = 2_000, value_parser = clap::value_parser!(u64).range(0..=60_000))]
    shutdown_drain_ms: u64,
}

/// Offline maintenance of a stopped validator's chain database (TODO 8.4.5).
#[derive(Subcommand, Debug)]
enum DbCommand {
    /// Print the database's chain identity and committed tip as JSON (read-only)
    Inspect(DbPath),
    /// Check page checksums, recovery metadata and every committed block; exit 1 on a problem
    Verify {
        #[command(flatten)]
        db: DbPath,
        /// Also require the chain ID and genesis commitment of this genesis file
        #[arg(long)]
        genesis: Option<PathBuf>,
    },
    /// Reclaim free space in the database file; records are not changed
    Compact(DbPath),
    /// Copy the chain database to a new file (never overwrites). The signing guard is never
    /// backed up: restoring an older guard would allow double signing
    Backup {
        #[command(flatten)]
        db: DbPath,
        #[arg(long)]
        out: PathBuf,
    },
}

#[derive(clap::Args, Debug)]
struct DbPath {
    /// Directory holding the node's Redb database (as passed to `start`)
    #[arg(long, default_value = ".miraset_data")]
    data_dir: PathBuf,
}

impl DbPath {
    fn chain_file(&self) -> PathBuf {
        self.data_dir.join(CHAIN_DB_FILE)
    }
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Run a persistent validator from a genesis file until SIGINT or SIGTERM
    Start(Box<StartArgs>),
    /// Offline database maintenance (the validator must be stopped)
    #[command(subcommand)]
    Db(DbCommand),
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
    /// Create an RPC API token: prints JSON with the token (a full credential: hand it to the
    /// client over a secure channel and keep it out of logs) and the line to add to the node's
    /// --rpc-api-keys file (which stores only the token's SHA-256)
    RpcKey {
        /// Key name: 1-64 characters of [A-Za-z0-9_.-]; rate limits are kept per name
        #[arg(long)]
        name: String,
    },
    /// Print a signed transfer as JSON, ready to POST to /v1/transactions
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
    /// Offer a measured transfer/receipt load to devnet node RPCs and print a JSON report
    /// (TODO 8.5.3). Funds throwaway senders from the faucet key; devnet use only
    Loadgen(Box<LoadgenArgs>),
    /// Run the interactive in-memory devnet demo
    Demo,
}

#[derive(clap::Args, Debug)]
struct LoadgenArgs {
    /// Node RPC base URL (repeat once per node); the first funds senders and observes commits
    #[arg(long = "rpc", required = true)]
    rpc_urls: Vec<String>,
    /// Funded key that pays for the senders (the devnet faucet key)
    #[arg(long)]
    faucet_key: PathBuf,
    /// Genesis coordinator key that co-signs receipts (devnet: /keys/coordinator/coordinator.key);
    /// required unless --receipt-share is 0
    #[arg(long)]
    coordinator_key: Option<PathBuf>,
    #[arg(long, default_value_t = 32)]
    senders: usize,
    /// Offered items (transfers + receipts) per second across all nodes
    #[arg(long, default_value_t = 20.0)]
    rate: f64,
    /// Fraction of offered items that are compute receipts
    #[arg(long, default_value_t = 0.15)]
    receipt_share: f64,
    #[arg(long, default_value_t = 60)]
    duration_secs: u64,
    /// Initial seconds of the run excluded from the report
    #[arg(long, default_value_t = 10)]
    warmup_secs: u64,
    /// After the load stops, seconds to wait for admitted items to commit
    #[arg(long, default_value_t = 60)]
    drain_timeout_secs: u64,
    /// Amount the faucet sends each sender before the run
    #[arg(long, default_value_t = 10_000_000)]
    fund_amount: u64,
    /// Start the load this many seconds after launch (senders are funded first)
    #[arg(long, default_value_t = 0)]
    start_delay_secs: u64,
}

#[tokio::main]
async fn main() -> CliResult {
    let cli = Cli::parse();
    init_logging(cli.log_format);
    // Before any key is read: no core dumps, no same-user ptrace (key custody D4).
    process_hardening::disable_core_dumps()?;
    match cli.command {
        Commands::Keygen { out } => keygen(&out),
        Commands::Pubkey { key_file } => {
            let key = devnet::read_key_file(&key_file)?;
            println!("{}", hex::encode(key.verifying_key().to_bytes()));
            Ok(())
        }
        Commands::DevnetInit {
            out,
            validators,
            chain_id,
            p2p_addr_template,
            faucet_balance,
            keys_dir,
        } => devnet_init(&DevnetSpec {
            out_dir: &out,
            validators,
            chain_id,
            p2p_addr_template: &p2p_addr_template,
            faucet_balance,
            keys_dir: keys_dir.as_deref(),
        }),
        Commands::RpcKey { name } => {
            let (token, line) = rpc::access::generate_key(&name)
                .ok_or("key name must be 1-64 characters of [A-Za-z0-9_.-]")?;
            println!(
                "{}",
                serde_json::json!({ "name": name, "token": token, "key_file_line": line })
            );
            Ok(())
        }
        Commands::SignTransfer {
            key_file,
            to,
            amount,
            fee,
            nonce,
        } => sign_transfer(&key_file, &to, amount, fee, nonce),
        Commands::Signer {
            genesis,
            key_file,
            signing_guard,
            socket,
            init_signing_guard,
        } => {
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
                    tracing::error!(
                        target: SAFETY_LOG_TARGET,
                        exit_code = SIGNER_HALT_EXIT_CODE,
                        "signer safety halt: {reason}; exiting"
                    );
                    std::process::exit(SIGNER_HALT_EXIT_CODE);
                }
            }
        }
        Commands::Loadgen(args) => run_loadgen(&args).await,
        Commands::Demo => {
            run_devnet_demo().await;
            Ok(())
        }
        Commands::Start(args) => run_validator(&args).await,
        Commands::Db(command) => {
            run_db(&command).unwrap_or_else(|error| {
                eprintln!("error: {error}");
                std::process::exit(error.exit_code());
            });
            Ok(())
        }
    }
}

/// Environment variable holding the RPC API token `loadgen` sends with submissions.
const LOADGEN_API_KEY_ENV: &str = "MIRASET_RPC_API_KEY";

fn parse_positive_rate(value: &str) -> Result<f64, String> {
    match value.parse::<f64>() {
        Ok(rate) if rate.is_finite() && rate > 0.0 => Ok(rate),
        _ => Err("must be a positive number".to_string()),
    }
}

/// Submission access policy from the start flags (TODO 8.4.3).
fn rpc_access(cli: &StartArgs) -> Result<RpcAccess, Box<dyn std::error::Error>> {
    let keys = match &cli.rpc_api_keys {
        Some(path) => {
            let keys = ApiKeys::load(path)?;
            tracing::info!(keys = keys.len(), "RPC submissions require an API key");
            Some(keys)
        }
        None => {
            tracing::warn!(
                "RPC submissions are open: no --rpc-api-keys file (acceptable on a private devnet only)"
            );
            None
        }
    };
    let limit = RateLimit {
        rate_per_sec: cli.rpc_submit_rate,
        burst: cli.rpc_submit_burst,
    };
    Ok(RpcAccess::new(keys, Some(limit)))
}

/// Run one offline maintenance command and print its JSON report on stdout. Errors carry their
/// exit code (`MaintenanceError::exit_code`).
fn run_db(command: &DbCommand) -> Result<(), MaintenanceError> {
    let report = match command {
        DbCommand::Inspect(db) => to_json(&maintenance::inspect(&db.chain_file())?),
        DbCommand::Verify { db, genesis } => {
            let expected = genesis.as_deref().map(expected_chain).transpose()?;
            to_json(&maintenance::verify(&db.chain_file(), expected)?)
        }
        DbCommand::Compact(db) => to_json(&maintenance::compact(&db.chain_file())?),
        DbCommand::Backup { db, out } => to_json(&maintenance::backup(&db.chain_file(), out)?),
    };
    println!("{report}");
    Ok(())
}

fn expected_chain(path: &Path) -> Result<ExpectedChain, MaintenanceError> {
    let genesis = GenesisConfig::load(path)
        .map_err(|e| MaintenanceError::Io(format!("cannot load {}: {e}", path.display())))?;
    Ok(ExpectedChain {
        chain_id: genesis.chain_id,
        genesis_hash: genesis.state_commitment(),
    })
}

fn to_json(report: &impl serde::Serialize) -> String {
    serde_json::to_string_pretty(report).expect("maintenance reports serialize")
}

async fn run_loadgen(args: &LoadgenArgs) -> CliResult {
    let config = miraset_chain::loadgen::LoadConfig {
        rpc_urls: args
            .rpc_urls
            .iter()
            .map(|u| u.trim_end_matches('/').to_string())
            .collect(),
        faucet: devnet::read_key_file(&args.faucet_key)?,
        coordinator: args
            .coordinator_key
            .as_deref()
            .map(devnet::read_key_file)
            .transpose()?,
        senders: args.senders,
        rate: args.rate,
        receipt_share: args.receipt_share,
        duration: Duration::from_secs(args.duration_secs),
        warmup: Duration::from_secs(args.warmup_secs),
        drain_timeout: Duration::from_secs(args.drain_timeout_secs),
        fund_amount: args.fund_amount,
        start_delay: Duration::from_secs(args.start_delay_secs),
        run_id: rand::random(),
        // From the environment so the token does not show in the process list.
        api_key: std::env::var(LOADGEN_API_KEY_ENV)
            .ok()
            .filter(|token| !token.is_empty()),
    };
    let report = miraset_chain::loadgen::run(&config)
        .await
        .unwrap_or_else(|error| {
            eprintln!("error: {error}");
            std::process::exit(1);
        });
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
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
async fn run_validator(cli: &StartArgs) -> CliResult {
    let genesis_path = cli.genesis.as_path();
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
    let kv_store = RedbKVStore::open(cli.data_dir.join(CHAIN_DB_FILE))
        .map_err(|e| format!("cannot open database: {e}"))?
        .with_safety_halt(safety_halt.clone());
    let genesis_hash = genesis.state_commitment();
    let guard = open_signing_guard(cli, &kv_store, genesis.chain_id, &vk, genesis_hash)?;
    init_genesis_with_params(
        kv_store.clone(),
        genesis.chain_id,
        &genesis.validator_powers()?,
        &genesis.initial_accounts(),
        &genesis.protocol,
    );
    genesis.verify_store_validator_set(&kv_store)?;

    // Key custody D1-D3: a local key moves into a GuardedSigner that checks the guard before
    // signing a phase vote; a remote signer holds key and guard in its own process. Either way
    // GuardedNetwork keeps this node's guard as a second layer.
    // A remote signer counts its refusals in its own process, so only a local one is exported.
    let (signer, signer_refusals): (Arc<dyn ValidatorSigner>, _) = match node_key {
        NodeKey::Local(signing_key) => {
            let signer = GuardedSigner::new(*signing_key, guard.clone())?;
            let refusals = signer.refusals();
            (signer, Some(refusals))
        }
        NodeKey::Remote(remote) => (remote, None),
    };

    // Authenticated transport (TODO 8.4.1): a per-start Noise key attested by the validator key.
    // The signer may be remote (a blocking gRPC client), so attest off the async runtime.
    let identity = {
        let signer = Arc::clone(&signer);
        let chain_id = genesis.chain_id;
        tokio::task::spawn_blocking(move || TransportIdentity::new(signer, chain_id, genesis_hash))
            .await
            .map_err(|e| format!("transport identity task failed: {e}"))??
    };
    let peers = genesis.peer_addresses(&vk)?;
    let (tcp, runtime) = TcpNetwork::bind_with_hosts(identity, cli.p2p_addr, peers)?;
    let p2p_dropped = tcp.dropped_messages_counter();
    let p2p_rejections = tcp.rejections();
    let p2p_traffic = tcp.traffic();
    // Outermost layer: TcpNetwork delivers self-addressed votes inside `send`.
    let network = GuardedNetwork::new(tcp, guard, safety_halt.clone());
    let transport_refusals = network.refusals();

    let mut config = NodeConfig::devnet_with_signer(signer);
    config.chain_id = genesis.chain_id;
    config.expected_genesis_hash = Some(genesis_hash);
    config.safety_halt = safety_halt.clone();
    config.ready_max_commit_age = Duration::from_secs(cli.ready_max_commit_age_secs);
    config.min_block_interval = Duration::from_millis(cli.min_block_interval_ms);
    // A consensus parameter fixed at genesis (TODO 8.5.2 F2).
    config.epoch_length = genesis.protocol.consensus_epoch_length;
    let node = Arc::new(MirasetNode::try_start(
        config,
        kv_store,
        network,
        Mempool::new(),
    )?);
    node.metrics().attach_p2p_dropped(p2p_dropped);
    node.metrics().attach_p2p_rejections(p2p_rejections);
    node.metrics().runtime().attach_traffic(p2p_traffic);
    node.metrics().runtime().attach_storage(StorageFiles {
        chain_db: cli.data_dir.join(CHAIN_DB_FILE),
        signing_guard: Some(signing_guard_path(cli)),
    });
    node.metrics()
        .attach_vote_refusals(VoteGuardLayer::Transport, transport_refusals);
    if let Some(refusals) = signer_refusals {
        node.metrics()
            .attach_vote_refusals(VoteGuardLayer::Signer, refusals);
    }

    tracing::info!(
        chain_id = genesis.chain_id,
        validator = %hex::encode(vk.to_bytes()),
        validators = genesis.validators.len(),
        data_dir = %cli.data_dir.display(),
        p2p_addr = %cli.p2p_addr,
        rpc_addr = %cli.rpc_addr,
        // Consensus parameters every validator must share (TODO 8.5.2 F2).
        epoch_length = genesis.protocol.consensus_epoch_length,
        "starting MIRASET experimental/devnet validator (ADR-0016 Proposed)"
    );

    let access = rpc_access(cli)?;
    let mut rpc = tokio::spawn(start_rpc_server(
        Arc::clone(&node),
        genesis.chain_id,
        cli.rpc_addr,
        access,
    ));
    let outcome: CliResult = tokio::select! {
        signal = shutdown_signal() => signal.map_err(Into::into),
        reason = wait_for_safety_halt(&safety_halt) => exit_on_safety_halt(&reason),
        result = &mut rpc => match result {
            Ok(Ok(())) => Err("RPC server exited unexpectedly".into()),
            Ok(Err(e)) => Err(format!("RPC server failed: {e}").into()),
            Err(e) => Err(format!("RPC task failed: {e}").into()),
        },
    };

    let drain = if outcome.is_ok() {
        Duration::from_millis(cli.shutdown_drain_ms)
    } else {
        Duration::ZERO
    };
    shutdown_validator(node, rpc, runtime, &safety_halt, drain).await;
    outcome
}

/// No graceful shutdown after a safety halt: the replica thread may have panicked, and joining
/// it would panic again. Redb commits are atomic, so exiting here loses nothing durable.
fn exit_on_safety_halt(reason: &str) -> ! {
    tracing::error!(
        target: SAFETY_LOG_TARGET,
        exit_code = SAFETY_HALT_EXIT_CODE,
        "consensus safety halt: {reason}; exiting"
    );
    std::process::exit(SAFETY_HALT_EXIT_CODE);
}

/// Graceful stop (TODO 8.4.4): refuse admission and report not ready, keep serving probes for
/// the drain period, then stop RPC, consensus (joins replica threads; Redb commits are atomic)
/// and the transport. Mempool contents are not durable; their count is reported (best effort:
/// a submission racing the drain flag may still be admitted).
///
/// A safety halt during the drain still exits with code 3, and a second SIGTERM/SIGINT ends the
/// drain early.
async fn shutdown_validator<K, N>(
    node: Arc<MirasetNode<K, N>>,
    rpc: tokio::task::JoinHandle<std::io::Result<()>>,
    runtime: miraset_chain::network::tcp::NetworkRuntime,
    safety_halt: &SafetyHalt,
    drain: Duration,
) where
    K: hotstuff_rs::block_tree::pluggables::KVStore,
    N: hotstuff_rs::networking::network::Network + 'static,
{
    node.lifecycle().begin_drain();
    tracing::info!(
        drain_ms = drain.as_millis() as u64,
        "shutdown: admission stopped, readiness withdrawn"
    );
    if !drain.is_zero() {
        tokio::select! {
            () = tokio::time::sleep(drain) => {}
            reason = wait_for_safety_halt(safety_halt) => exit_on_safety_halt(&reason),
            _ = shutdown_signal() => tracing::warn!("shutdown: second signal, ending drain early"),
        }
    }
    if let Some(reason) = safety_halt.reason() {
        exit_on_safety_halt(reason);
    }
    rpc.abort();
    let _ = rpc.await;
    let pending_txs = node.mempool().tx_count();
    let pending_receipts = node.mempool().receipt_count();
    if pending_txs + pending_receipts > 0 {
        tracing::warn!(
            pending_txs,
            pending_receipts,
            "shutdown: unfinalized mempool items are discarded (the mempool is not durable); clients must resubmit"
        );
    }
    let height = node.highest_committed_height();
    drop(node);
    drop(runtime);
    tracing::info!(committed_height = ?height, "shutdown complete");
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

async fn load_node_key(cli: &StartArgs, chain_id: u64) -> CliResult<NodeKey> {
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
        _ => Err("`start` needs exactly one of --key-file or --remote-signer".into()),
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

/// Where the vote signing guard lives: `--signing-guard`, or `<data dir>/signing-guard.redb`.
fn signing_guard_path(cli: &StartArgs) -> std::path::PathBuf {
    cli.signing_guard
        .clone()
        .unwrap_or_else(|| cli.data_dir.join(SIGNING_GUARD_FILE))
}

/// Open the vote signing guard (TODO 8.3.2). A non-empty chain database requires an existing
/// guard unless `--init-signing-guard` is given, so a lost guard is never silently recreated.
fn open_signing_guard(
    cli: &StartArgs,
    kv_store: &RedbKVStore,
    chain_id: u64,
    vk: &ed25519_dalek::VerifyingKey,
    genesis_hash: [u8; 32],
) -> CliResult<SigningGuard> {
    let path = signing_guard_path(cli);
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
    tracing::info!(
        "signing guard {} opened (last signed view: {})",
        path.display(),
        last_view.map_or_else(|| "none".to_string(), |view| view.to_string())
    );
    Ok(guard)
}

/// Resolve once the safety halt fires, after a grace period that lets `/v1/health` report it.
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
    let worker_owner_key = SigningKey::from_bytes(&[9u8; 32]);
    let worker_owner = hex::encode(worker_owner_key.verifying_key().to_bytes());
    // TODO 8.5.1: receipts need a genesis coordinator and a registered, bonded worker.
    let params = ProtocolParams {
        coordinators: vec![hex::encode(coordinator_key.verifying_key().to_bytes())],
        price_per_token: 2,
        ..ProtocolParams::default()
    };

    let validators = vec![(vk, 100)];
    let initial_accounts = vec![
        (alice_addr.as_str(), 1_000_000),
        ("bob", 250_000),
        (worker_owner.as_str(), 10_000),
    ];

    println!("\n[2] Initializing Genesis Accounts:");
    for (acc, bal) in &initial_accounts {
        println!("    - Account '{}': {} MIRA", acc, bal);
    }
    println!("    (Account '{alice_addr}' is addressed below as 'alice')");
    init_genesis_with_params(
        kv_store.clone(),
        1337,
        &validators,
        &initial_accounts,
        &params,
    );

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
        if let Err(e) = start_rpc_server(rpc_node, chain_id, rpc_addr, RpcAccess::open()).await {
            eprintln!("RPC server error: {}", e);
        }
    });
    println!("    ✓ Node RPC API listening on http://{}", rpc_addr);
    println!(
        "      Endpoints: /v1/health, /v1/accounts/:addr, /v1/workers/:id, /v1/tasks/:id, /v1/transactions, /v1/receipts, /v1/blocks/latest, /v1/events/blocks"
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

    // 7. Register a worker with the minimum bond, then settle a receipt it and the coordinator
    // signed: 250 tokens at price 2, paid by Alice to the worker's owner.
    println!("\n[5] Registering worker_4090 and submitting a PoCC compute receipt:");
    let register = WorkerBondAction::Register {
        worker_id: "worker_4090".into(),
        bond_amount: MIN_WORKER_BOND,
        vram_gib: 24,
        endpoint: String::new(),
    };
    let mut bond_tx = Transaction::new_worker_bond(&worker_owner, 0, 10, register)
        .expect("worker bond transaction");
    bond_tx.sign_with(&worker_owner_key);
    node.submit_transaction(bond_tx)
        .expect("Worker registration failed");
    // Alice allows the coordinator to charge her up to 1 000 (her nonce 1; nonce 0 was the
    // transfer above).
    let mut allow = Transaction::new(&alice_addr, 1, "system:allowance", 0, 10);
    allow.payload = AllowanceAction {
        coordinator: hex::encode(coordinator_key.verifying_key().to_bytes()),
        amount: 1_000,
    }
    .to_bytes()
    .expect("allowance payload");
    allow.sign_with(&alice_key);
    node.submit_transaction(allow)
        .expect("Allowance submission failed");
    let coordinator_hex = hex::encode(coordinator_key.verifying_key().to_bytes());
    for _ in 0..100 {
        if node.get_worker("worker_4090").is_some()
            && node.allowance(&alice_addr, &coordinator_hex) > 0
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let mut receipt1 = ComputeReceipt::new(
        "inference_task_gpt_oss_1",
        &alice_addr,
        "worker_4090",
        "coordinator_east",
        "sha256:7f83b1657ff1fc53b92dc18148a1d65dfc2d4b1fa3d677284addd200126d9069",
        "receipt-anchor:0x892a71f0923058b73a4cf02",
        250,
        500,
        500,
    );
    receipt1.sign_with(&coordinator_key);
    receipt1.sign_as_worker(&worker_owner_key);
    println!(
        "    - Worker: {} completed task '{}' (250 tokens; Alice pays 500 to the worker's owner)",
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
