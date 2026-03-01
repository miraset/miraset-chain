"use client";

import { invoke } from "@tauri-apps/api/tauri";
import { useCallback, useMemo, useState, useSyncExternalStore } from "react";

const DEFAULT_RPC_URL = "http://127.0.0.1:9944";
const DEFAULT_OLLAMA_URL = "http://127.0.0.1:11434";
const DEFAULT_WORKER_URL = "http://127.0.0.1:8080";

type Account = {
  name: string;
  address: string;
  balance?: number;
};

type AppConfig = {
  rpc_url: string;
  wallet_path: string;
};

type Status = {
  kind: "idle" | "loading" | "error" | "success";
  message: string;
};

type ConnectionState = "unknown" | "checking" | "online" | "offline";

type ConnectionStatus = {
  state: ConnectionState;
  detail?: string;
};

type ConnectionSnapshot = {
  rpc: ConnectionStatus;
  ollama: ConnectionStatus;
  worker: ConnectionStatus;
  lastChecked?: number;
};

const DEFAULT_CONNECTION_SNAPSHOT: ConnectionSnapshot = {
  rpc: { state: "unknown" },
  ollama: { state: "unknown" },
  worker: { state: "unknown" },
};

let connectionSnapshot: ConnectionSnapshot = DEFAULT_CONNECTION_SNAPSHOT;
const connectionSubscribers = new Set<() => void>();
let connectionTimer: number | null = null;
let connectionTargets = {
  rpcUrl: DEFAULT_RPC_URL,
  ollamaUrl: DEFAULT_OLLAMA_URL,
  workerUrl: DEFAULT_WORKER_URL,
};

function notifyConnectionSubscribers() {
  connectionSubscribers.forEach((listener) => listener());
}

async function checkConnectionsOnce() {
  if (typeof window === "undefined") {
    return;
  }
  if (!detectTauri()) {
    connectionSnapshot = DEFAULT_CONNECTION_SNAPSHOT;
    notifyConnectionSubscribers();
    return;
  }
  connectionSnapshot = {
    ...connectionSnapshot,
    rpc: { state: "checking" },
    ollama: { state: "checking" },
    worker: { state: "checking" },
  };
  notifyConnectionSubscribers();

  try {
    const result = await invoke<ConnectionSnapshot>("check_connections", {
      rpcUrl: connectionTargets.rpcUrl,
      workerUrl: connectionTargets.workerUrl,
      ollamaUrl: connectionTargets.ollamaUrl,
    });
    connectionSnapshot = {
      ...result,
      lastChecked: Date.now(),
    };
  } catch (error) {
    const detail = formatErrorMessage(error, "unreachable");
    connectionSnapshot = {
      rpc: { state: "offline", detail },
      worker: { state: "offline", detail },
      ollama: { state: "offline", detail },
      lastChecked: Date.now(),
    };
  }
  notifyConnectionSubscribers();
}

function startConnectionPolling() {
  if (connectionTimer !== null || typeof window === "undefined") {
    return;
  }
  void checkConnectionsOnce();
  connectionTimer = window.setInterval(() => {
    void checkConnectionsOnce();
  }, 10000);
}

function stopConnectionPolling() {
  if (connectionTimer === null) {
    return;
  }
  window.clearInterval(connectionTimer);
  connectionTimer = null;
}

function subscribeConnections(listener: () => void) {
  connectionSubscribers.add(listener);
  if (connectionSubscribers.size === 1) {
    startConnectionPolling();
  }
  return () => {
    connectionSubscribers.delete(listener);
    if (connectionSubscribers.size === 0) {
      stopConnectionPolling();
    }
  };
}

function getConnectionSnapshot() {
  return connectionSnapshot;
}

function updateConnectionTargets(
  rpcUrl: string,
  ollamaUrl: string,
  workerUrl: string
) {
  connectionTargets = {
    rpcUrl: rpcUrl || DEFAULT_RPC_URL,
    ollamaUrl: ollamaUrl || DEFAULT_OLLAMA_URL,
    workerUrl: workerUrl || DEFAULT_WORKER_URL,
  };
  void checkConnectionsOnce();
}

function detectTauri() {
  return (
    typeof window !== "undefined" &&
    typeof (window as { __TAURI_IPC__?: unknown }).__TAURI_IPC__ !== "undefined"
  );
}


function formatErrorMessage(error: unknown, fallback: string) {
  if (error instanceof Error && error.message) {
    return error.message;
  }
  if (typeof error === "string") {
    return error;
  }
  try {
    return JSON.stringify(error);
  } catch {
    return fallback;
  }
}

export default function Home() {
  const [accounts, setAccounts] = useState<Account[]>([]);
  const [selectedAccount, setSelectedAccount] = useState<string>("");
  const [config, setConfig] = useState<AppConfig | null>(null);
  const [status, setStatus] = useState<Status>({ kind: "idle", message: "" });
  const [accountsLoaded, setAccountsLoaded] = useState(false);

  const connectionSnapshot = useSyncExternalStore(
    subscribeConnections,
    getConnectionSnapshot,
    getConnectionSnapshot
  );
  const rpcStatus = connectionSnapshot.rpc;
  const ollamaStatus = connectionSnapshot.ollama;
  const workerStatus = connectionSnapshot.worker;

  const isHydrated = useSyncExternalStore(
    () => () => undefined,
    () => true,
    () => false
  );

  const isTauri = useSyncExternalStore(
    () => () => undefined,
    () => detectTauri(),
    () => false
  );

  const tauriInvoke = useCallback(
    async <T,>(cmd: string, args?: Record<string, unknown>) => {
      if (!detectTauri()) {
        throw new Error("Tauri runtime not available. Start the desktop app.");
      }
      return invoke<T>(cmd, args);
    },
    []
  );

  const [newAccountName, setNewAccountName] = useState("");
  const [importName, setImportName] = useState("");
  const [importSecret, setImportSecret] = useState("");
  const [exportSecret, setExportSecret] = useState("");

  const [transferTo, setTransferTo] = useState("");
  const [transferAmount, setTransferAmount] = useState("0");

  const [rpcUrlDraft, setRpcUrlDraft] = useState("");

  const selectedAddress = useMemo(() => {
    const account = accounts.find((item) => item.name === selectedAccount);
    return account?.address ?? "";
  }, [accounts, selectedAccount]);

  const selectedBalance = useMemo(() => {
    const account = accounts.find((item) => item.name === selectedAccount);
    return account?.balance ?? 0;
  }, [accounts, selectedAccount]);

  const refreshAll = useCallback(async () => {
    setStatus({ kind: "loading", message: "Loading wallet data..." });
    try {
      const loadedConfig = await tauriInvoke<AppConfig>("get_config");
      setConfig(loadedConfig);
      setRpcUrlDraft(loadedConfig.rpc_url);

      updateConnectionTargets(
        loadedConfig.rpc_url || DEFAULT_RPC_URL,
        DEFAULT_OLLAMA_URL,
        DEFAULT_WORKER_URL
      );

      const accountsList = await tauriInvoke<Account[]>("list_accounts");
      const enriched = await Promise.all(
        accountsList.map(async (item) => {
          const balance = await tauriInvoke<number>("get_balance", {
            address: item.address,
          });
          return { ...item, balance };
        })
      );
      setAccounts(enriched);
      setAccountsLoaded(true);
      if (!selectedAccount && enriched.length > 0) {
        setSelectedAccount(enriched[0].name);
      }
      setStatus({ kind: "success", message: "Wallet data updated." });
    } catch (error) {
      setStatus({
        kind: "error",
        message: formatErrorMessage(error, "Unexpected error"),
      });
    }
  }, [selectedAccount, tauriInvoke]);

  async function handleCreateAccount() {
    if (!newAccountName.trim()) {
      setStatus({ kind: "error", message: "Account name is required." });
      return;
    }
    setStatus({ kind: "loading", message: "Creating account..." });
    try {
      await tauriInvoke<string>("create_account", { name: newAccountName.trim() });
      setNewAccountName("");
      await refreshAll();
    } catch (error) {
      setStatus({
        kind: "error",
        message: formatErrorMessage(error, "Failed to create account"),
      });
    }
  }

  async function handleImportAccount() {
    if (!importName.trim() || !importSecret.trim()) {
      setStatus({
        kind: "error",
        message: "Import name and secret key are required.",
      });
      return;
    }
    setStatus({ kind: "loading", message: "Importing account..." });
    try {
      await tauriInvoke<string>("import_account", {
        name: importName.trim(),
        secret_hex: importSecret.trim(),
      });
      setImportName("");
      setImportSecret("");
      await refreshAll();
    } catch (error) {
      setStatus({
        kind: "error",
        message: formatErrorMessage(error, "Failed to import account"),
      });
    }
  }

  async function handleExportSecret() {
    if (!selectedAccount) {
      setStatus({ kind: "error", message: "Select an account first." });
      return;
    }
    setStatus({ kind: "loading", message: "Exporting secret..." });
    try {
      const secret = await tauriInvoke<string>("export_secret", {
        name: selectedAccount,
      });
      setExportSecret(secret);
      setStatus({
        kind: "success",
        message: "Secret exported. Keep it secure!",
      });
    } catch (error) {
      setStatus({
        kind: "error",
        message: formatErrorMessage(error, "Failed to export secret"),
      });
    }
  }

  async function handleTransfer() {
    if (!selectedAccount) {
      setStatus({ kind: "error", message: "Select a sender account." });
      return;
    }
    const amount = Number(transferAmount);
    if (!transferTo.trim() || Number.isNaN(amount) || amount <= 0) {
      setStatus({
        kind: "error",
        message: "Enter a valid recipient and amount.",
      });
      return;
    }
    setStatus({ kind: "loading", message: "Submitting transfer..." });
    try {
      await tauriInvoke<void>("transfer", {
        from: selectedAccount,
        to: transferTo.trim(),
        amount,
      });
      setTransferTo("");
      setTransferAmount("0");
      await refreshAll();
    } catch (error) {
      setStatus({
        kind: "error",
        message: formatErrorMessage(error, "Transfer failed"),
      });
    }
  }

  async function handleUpdateRpc() {
    if (!rpcUrlDraft.trim()) {
      setStatus({ kind: "error", message: "RPC URL is required." });
      return;
    }
    setStatus({ kind: "loading", message: "Updating RPC URL..." });
    try {
      const updated = await tauriInvoke<AppConfig>("set_rpc_url", {
        rpc_url: rpcUrlDraft.trim(),
      });
      setConfig(updated);
      updateConnectionTargets(
        updated.rpc_url || DEFAULT_RPC_URL,
        DEFAULT_OLLAMA_URL,
        DEFAULT_WORKER_URL
      );
      setStatus({ kind: "success", message: "RPC URL updated." });
    } catch (error) {
      setStatus({
        kind: "error",
        message: formatErrorMessage(error, "Failed to update RPC URL"),
      });
    }
  }

  async function handleCopyAddress() {
    if (!selectedAddress) {
      return;
    }
    await navigator.clipboard.writeText(selectedAddress);
    setStatus({ kind: "success", message: "Address copied to clipboard." });
  }

  if (!isHydrated) {
    return (
      <div className="min-h-screen bg-[#0b0d10] text-zinc-100">
        <div className="mx-auto flex max-w-6xl flex-col gap-6 px-6 py-10">
          <header className="flex flex-col gap-2">
            <h1 className="font-display text-4xl">MIRASET Wallet</h1>
            <p className="text-sm text-zinc-400">Loading...</p>
          </header>
        </div>
      </div>
    );
  }

  if (!isTauri) {
    return (
      <div className="flex min-h-screen items-center justify-center bg-[#0b0d10] text-white">
        <div className="max-w-xl rounded-lg border border-[#24262c] bg-[#14161a] p-8 text-center">
          <h1 className="font-display text-3xl">MIRASET Wallet</h1>
          <p className="mt-3 text-sm text-zinc-300">
            This UI runs inside the desktop app. Use the installer or run
            <span className="font-semibold"> bunx tauri dev</span>.
          </p>
        </div>
      </div>
    );
  }

  return (
    <div className="min-h-screen bg-[#0b0d10] text-zinc-100">
      <div className="mx-auto flex max-w-6xl flex-col gap-6 px-6 py-10">
        <header className="flex flex-col gap-2">
          <h1 className="font-display text-4xl">MIRASET Wallet</h1>
          <p className="text-sm text-zinc-400">
            Desktop wallet connected to the MIRASET RPC.
          </p>
          <div className="mt-2 flex flex-wrap items-center gap-3 text-xs text-zinc-400">
            <span className="font-semibold uppercase tracking-[0.2em] text-zinc-500">
              Status
            </span>
            <span className="flex items-center gap-2 rounded-full border border-[#24262c] px-3 py-1">
              <span
                className={`h-2 w-2 rounded-full ${
                  rpcStatus.state === "online"
                    ? "bg-emerald-400"
                    : rpcStatus.state === "offline"
                    ? "bg-red-400"
                    : rpcStatus.state === "checking"
                    ? "bg-amber-400"
                    : "bg-zinc-600"
                }`}
              />
              MIRASET RPC
            </span>
            <span className="flex items-center gap-2 rounded-full border border-[#24262c] px-3 py-1">
              <span
                className={`h-2 w-2 rounded-full ${
                  workerStatus.state === "online"
                    ? "bg-emerald-400"
                    : workerStatus.state === "offline"
                    ? "bg-red-400"
                    : workerStatus.state === "checking"
                    ? "bg-amber-400"
                    : "bg-zinc-600"
                }`}
              />
              MIRASET Worker
            </span>
            <span className="flex items-center gap-2 rounded-full border border-[#24262c] px-3 py-1">
              <span
                className={`h-2 w-2 rounded-full ${
                  ollamaStatus.state === "online"
                    ? "bg-emerald-400"
                    : ollamaStatus.state === "offline"
                    ? "bg-red-400"
                    : ollamaStatus.state === "checking"
                    ? "bg-amber-400"
                    : "bg-zinc-600"
                }`}
              />
              Ollama
            </span>
            {connectionSnapshot.lastChecked && (
              <span className="text-zinc-500">
                Checked {new Date(connectionSnapshot.lastChecked).toLocaleTimeString()}
              </span>
            )}
          </div>
        </header>

        {rpcStatus.state === "offline" && (
          <section className="rounded-md border border-red-500/30 bg-red-500/10 px-4 py-2 text-sm text-red-200">
            MIRASET RPC is offline. Start the node or update the RPC URL.
          </section>
        )}
        {workerStatus.state === "offline" && (
          <section className="rounded-md border border-amber-500/30 bg-amber-500/10 px-4 py-2 text-sm text-amber-200">
            MIRASET Worker is offline. Start the worker on port 8080.
          </section>
        )}

        <section className="rounded-xl border border-[#24262c] bg-[#14161a] p-6 shadow-lg">
          <div className="flex flex-col gap-4 md:flex-row md:items-end md:justify-between">
            <div className="flex-1">
              <label className="text-xs font-semibold uppercase tracking-[0.2em] text-zinc-400">
                RPC URL
              </label>
              <input
                className="mt-2 w-full rounded-md border border-[#24262c] bg-[#0f1115] px-3 py-2 text-sm text-zinc-100 placeholder:text-zinc-500"
                value={rpcUrlDraft}
                onChange={(event) => setRpcUrlDraft(event.target.value)}
                placeholder="http://127.0.0.1:9944"
              />
            </div>
            <div className="flex gap-2">
              <button
                className="rounded-md bg-[#f7931a] px-4 py-2 text-sm font-semibold text-black"
                onClick={handleUpdateRpc}
              >
                Save
              </button>
              <button
                className="rounded-md border border-[#24262c] px-4 py-2 text-sm text-zinc-200"
                onClick={refreshAll}
              >
                Refresh
              </button>
            </div>
          </div>
          <div className="mt-4 grid gap-2 text-xs text-zinc-400 md:grid-cols-3">
            <div className="flex items-center gap-2">
              <span
                className={`h-2.5 w-2.5 rounded-full ${
                  rpcStatus.state === "online"
                    ? "bg-emerald-400"
                    : rpcStatus.state === "offline"
                    ? "bg-red-400"
                    : rpcStatus.state === "checking"
                    ? "bg-amber-400"
                    : "bg-zinc-600"
                }`}
              />
              <span className="font-semibold text-zinc-200">MIRASET RPC</span>
              <span className="text-zinc-500">
                {rpcStatus.state}
                {rpcStatus.detail ? ` • ${rpcStatus.detail}` : ""}
              </span>
            </div>
            <div className="flex items-center gap-2">
              <span
                className={`h-2.5 w-2.5 rounded-full ${
                  workerStatus.state === "online"
                    ? "bg-emerald-400"
                    : workerStatus.state === "offline"
                    ? "bg-red-400"
                    : workerStatus.state === "checking"
                    ? "bg-amber-400"
                    : "bg-zinc-600"
                }`}
              />
              <span className="font-semibold text-zinc-200">MIRASET Worker</span>
              <span className="text-zinc-500">
                {workerStatus.state}
                {workerStatus.detail ? ` • ${workerStatus.detail}` : ""}
              </span>
            </div>
            <div className="flex items-center gap-2">
              <span
                className={`h-2.5 w-2.5 rounded-full ${
                  ollamaStatus.state === "online"
                    ? "bg-emerald-400"
                    : ollamaStatus.state === "offline"
                    ? "bg-red-400"
                    : ollamaStatus.state === "checking"
                    ? "bg-amber-400"
                    : "bg-zinc-600"
                }`}
              />
              <span className="font-semibold text-zinc-200">Ollama</span>
              <span className="text-zinc-500">
                {ollamaStatus.state}
                {ollamaStatus.detail ? ` • ${ollamaStatus.detail}` : ""}
              </span>
            </div>
          </div>
          <p className="mt-3 text-xs text-zinc-500">
            Wallet file: {config?.wallet_path ?? "loading..."}
          </p>
        </section>

        <section className="grid gap-6 lg:grid-cols-[1.1fr_1fr]">
          <div className="rounded-xl border border-[#24262c] bg-[#14161a] p-6">
            <div className="flex items-center justify-between">
              <h2 className="font-display text-2xl">Accounts</h2>
              <span className="text-xs text-zinc-500">{accounts.length} total</span>
            </div>
            <div className="mt-4 grid gap-3">
              {accounts.map((account) => (
                <button
                  key={account.name}
                  className={`flex flex-col rounded-md border px-3 py-3 text-left text-sm transition ${
                    selectedAccount === account.name
                      ? "border-[#f7931a] bg-[#0f1115]"
                      : "border-[#24262c] bg-[#0f1115]/60"
                  }`}
                  onClick={() => setSelectedAccount(account.name)}
                >
                  <span className="font-semibold text-zinc-100">{account.name}</span>
                  <span className="text-xs text-zinc-500">{account.address}</span>
                  <span className="mt-1 text-xs text-zinc-300">
                    Balance: {account.balance ?? 0} SECCO
                  </span>
                </button>
              ))}
              {!accountsLoaded && (
                <div className="rounded-md border border-dashed border-[#24262c] bg-[#0f1115]/50 px-3 py-4 text-sm text-zinc-400">
                  Existing wallets are not loaded by default.
                  <button
                    className="mt-3 inline-flex rounded-md bg-[#f7931a] px-3 py-2 text-xs font-semibold text-black"
                    onClick={refreshAll}
                  >
                    Load existing wallets
                  </button>
                </div>
              )}
              {accountsLoaded && accounts.length === 0 && (
                <p className="text-sm text-zinc-500">No accounts yet.</p>
              )}
            </div>

            <div className="mt-6 grid gap-3">
              <div>
                <label className="text-xs font-semibold uppercase tracking-[0.2em] text-zinc-400">
                  New account name
                </label>
                <div className="mt-2 flex gap-2">
                  <input
                    className="flex-1 rounded-md border border-[#24262c] bg-[#0f1115] px-3 py-2 text-sm text-zinc-100"
                    value={newAccountName}
                    onChange={(event) => setNewAccountName(event.target.value)}
                    placeholder="alice"
                  />
                  <button
                    className="rounded-md bg-[#f7931a] px-4 py-2 text-sm font-semibold text-black"
                    onClick={handleCreateAccount}
                  >
                    Create
                  </button>
                </div>
              </div>

              <div>
                <label className="text-xs font-semibold uppercase tracking-[0.2em] text-zinc-400">
                  Import account
                </label>
                <div className="mt-2 grid gap-2">
                  <input
                    className="rounded-md border border-[#24262c] bg-[#0f1115] px-3 py-2 text-sm text-zinc-100"
                    value={importName}
                    onChange={(event) => setImportName(event.target.value)}
                    placeholder="name"
                  />
                  <input
                    className="rounded-md border border-[#24262c] bg-[#0f1115] px-3 py-2 text-sm text-zinc-100"
                    value={importSecret}
                    onChange={(event) => setImportSecret(event.target.value)}
                    placeholder="secret hex"
                  />
                  <button
                    className="rounded-md border border-[#24262c] px-4 py-2 text-sm text-zinc-200"
                    onClick={handleImportAccount}
                  >
                    Import
                  </button>
                </div>
              </div>
            </div>
          </div>

          <div className="flex flex-col gap-6">
            <div className="rounded-xl border border-[#24262c] bg-[#14161a] p-6">
              <h2 className="font-display text-2xl">Selected Account</h2>
              <p className="mt-2 text-sm text-zinc-500">
                {selectedAccount
                  ? `${selectedAccount} • ${selectedAddress}`
                  : "Select an account to continue."}
              </p>
              <p className="mt-2 text-sm text-zinc-300">
                Balance: {selectedBalance} SECCO
              </p>
              <div className="mt-4 flex flex-wrap gap-2">
                <button
                  className="rounded-md border border-[#24262c] px-4 py-2 text-sm text-zinc-200"
                  onClick={handleCopyAddress}
                >
                  Copy Address
                </button>
                <button
                  className="rounded-md border border-[#24262c] px-4 py-2 text-sm text-zinc-200"
                  onClick={handleExportSecret}
                >
                  Export Secret
                </button>
              </div>
              {exportSecret && (
                <div className="mt-3 rounded-md border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-xs text-amber-200">
                  Secret key: {exportSecret}
                </div>
              )}
            </div>

            <div className="rounded-xl border border-[#24262c] bg-[#14161a] p-6">
              <h2 className="font-display text-2xl">Send SECCO</h2>
              <div className="mt-3 grid gap-2">
                <input
                  className="rounded-md border border-[#24262c] bg-[#0f1115] px-3 py-2 text-sm text-zinc-100"
                  value={transferTo}
                  onChange={(event) => setTransferTo(event.target.value)}
                  placeholder="Recipient address"
                />
                <input
                  className="rounded-md border border-[#24262c] bg-[#0f1115] px-3 py-2 text-sm text-zinc-100"
                  value={transferAmount}
                  onChange={(event) => setTransferAmount(event.target.value)}
                  placeholder="Amount"
                  type="number"
                  min="0"
                />
                <button
                  className="rounded-md bg-[#f7931a] px-4 py-2 text-sm font-semibold text-black"
                  onClick={handleTransfer}
                >
                  Send
                </button>
              </div>
            </div>

            <div className="rounded-xl border border-[#24262c] bg-[#14161a] p-6">
              <h2 className="font-display text-2xl">Receive SECCO</h2>
              <p className="mt-2 text-sm text-zinc-500">
                Share this address with the sender:
              </p>
              <div className="mt-2 rounded-md border border-dashed border-[#24262c] bg-[#0f1115] px-3 py-2 text-xs text-zinc-300">
                {selectedAddress || "Select an account."}
              </div>
            </div>
          </div>
        </section>

        {status.message && (
          <section
            className={`rounded-md px-4 py-2 text-sm ${
              status.kind === "error"
                ? "bg-red-500/10 text-red-300"
                : status.kind === "success"
                ? "bg-emerald-500/10 text-emerald-300"
                : "bg-[#14161a] text-zinc-300"
            }`}
          >
            {status.message}
          </section>
        )}
      </div>
    </div>
  );
}
