"use client";

import { invoke } from "@tauri-apps/api/tauri";
import { useEffect, useMemo, useState } from "react";

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

function detectTauri() {
  return (
    typeof window !== "undefined" &&
    typeof (window as { __TAURI_IPC__?: unknown }).__TAURI_IPC__ !== "undefined"
  );
}

export default function Home() {
  const [accounts, setAccounts] = useState<Account[]>([]);
  const [selectedAccount, setSelectedAccount] = useState<string>("");
  const [config, setConfig] = useState<AppConfig | null>(null);
  const [status, setStatus] = useState<Status>({ kind: "idle", message: "" });
  const [isHydrated, setIsHydrated] = useState(false);
  const [isTauri, setIsTauri] = useState(false);

  async function tauriInvoke<T>(cmd: string, args?: Record<string, unknown>) {
    if (!detectTauri()) {
      throw new Error("Tauri runtime not available. Start the desktop app.");
    }
    return invoke<T>(cmd, args);
  }

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

  useEffect(() => {
    setIsHydrated(true);
    setIsTauri(detectTauri());
  }, []);

  useEffect(() => {
    if (!isTauri) {
      return;
    }
    void refreshAll();
  }, [isTauri]);

  async function refreshAll() {
    setStatus({ kind: "loading", message: "Loading wallet data..." });
    try {
      const loadedConfig = await tauriInvoke<AppConfig>("get_config");
      setConfig(loadedConfig);
      setRpcUrlDraft(loadedConfig.rpc_url);

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
      if (!selectedAccount && enriched.length > 0) {
        setSelectedAccount(enriched[0].name);
      }
      setStatus({ kind: "success", message: "Wallet data updated." });
    } catch (error) {
      setStatus({
        kind: "error",
        message: error instanceof Error ? error.message : "Unexpected error",
      });
    }
  }

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
        message: error instanceof Error ? error.message : "Failed to create account",
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
        message: error instanceof Error ? error.message : "Failed to import account",
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
        message: error instanceof Error ? error.message : "Failed to export secret",
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
        message: error instanceof Error ? error.message : "Transfer failed",
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
      setStatus({ kind: "success", message: "RPC URL updated." });
    } catch (error) {
      setStatus({
        kind: "error",
        message: error instanceof Error ? error.message : "Failed to update RPC URL",
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
      <div className="min-h-screen bg-zinc-50 text-zinc-900">
        <div className="mx-auto flex max-w-6xl flex-col gap-6 px-6 py-8">
          <header className="flex flex-col gap-2">
            <h1 className="text-3xl font-semibold">MIRASET Wallet</h1>
            <p className="text-sm text-zinc-600">Loading...</p>
          </header>
        </div>
      </div>
    );
  }

  if (!isTauri) {
    return (
      <div className="flex min-h-screen items-center justify-center bg-zinc-950 text-white">
        <div className="max-w-xl rounded-lg border border-zinc-800 bg-zinc-900 p-8 text-center">
          <h1 className="text-2xl font-semibold">MIRASET Wallet</h1>
          <p className="mt-3 text-sm text-zinc-300">
            This UI runs inside the desktop app. Use the installer or run
            <span className="font-semibold"> bunx tauri dev</span>.
          </p>
        </div>
      </div>
    );
  }

  return (
    <div className="min-h-screen bg-zinc-50 text-zinc-900">
      <div className="mx-auto flex max-w-6xl flex-col gap-6 px-6 py-8">
        <header className="flex flex-col gap-2">
          <h1 className="text-3xl font-semibold">MIRASET Wallet</h1>
          <p className="text-sm text-zinc-600">
            Desktop wallet connected to the MIRASET RPC.
          </p>
        </header>

        <section className="rounded-lg border border-zinc-200 bg-white p-5">
          <div className="flex flex-col gap-4 md:flex-row md:items-end md:justify-between">
            <div className="flex-1">
              <label className="text-xs font-semibold uppercase text-zinc-500">
                RPC URL
              </label>
              <input
                className="mt-2 w-full rounded-md border border-zinc-200 px-3 py-2 text-sm"
                value={rpcUrlDraft}
                onChange={(event) => setRpcUrlDraft(event.target.value)}
                placeholder="http://127.0.0.1:9944"
              />
            </div>
            <div className="flex gap-2">
              <button
                className="rounded-md bg-zinc-900 px-4 py-2 text-sm font-semibold text-white"
                onClick={handleUpdateRpc}
              >
                Save
              </button>
              <button
                className="rounded-md border border-zinc-200 px-4 py-2 text-sm"
                onClick={refreshAll}
              >
                Refresh
              </button>
            </div>
          </div>
          <p className="mt-3 text-xs text-zinc-500">
            Wallet file: {config?.wallet_path ?? "loading..."}
          </p>
        </section>

        <section className="grid gap-6 lg:grid-cols-[1.1fr_1fr]">
          <div className="rounded-lg border border-zinc-200 bg-white p-5">
            <div className="flex items-center justify-between">
              <h2 className="text-lg font-semibold">Accounts</h2>
              <span className="text-xs text-zinc-500">
                {accounts.length} total
              </span>
            </div>
            <div className="mt-4 grid gap-3">
              {accounts.map((account) => (
                <button
                  key={account.name}
                  className={`flex flex-col rounded-md border px-3 py-2 text-left text-sm transition ${
                    selectedAccount === account.name
                      ? "border-zinc-900 bg-zinc-50"
                      : "border-zinc-200"
                  }`}
                  onClick={() => setSelectedAccount(account.name)}
                >
                  <span className="font-semibold">{account.name}</span>
                  <span className="text-xs text-zinc-500">{account.address}</span>
                  <span className="mt-1 text-xs text-zinc-700">
                    Balance: {account.balance ?? 0} MIRA
                  </span>
                </button>
              ))}
              {accounts.length === 0 && (
                <p className="text-sm text-zinc-500">No accounts yet.</p>
              )}
            </div>

            <div className="mt-6 grid gap-3">
              <div>
                <label className="text-xs font-semibold uppercase text-zinc-500">
                  New account name
                </label>
                <div className="mt-2 flex gap-2">
                  <input
                    className="flex-1 rounded-md border border-zinc-200 px-3 py-2 text-sm"
                    value={newAccountName}
                    onChange={(event) => setNewAccountName(event.target.value)}
                    placeholder="alice"
                  />
                  <button
                    className="rounded-md bg-zinc-900 px-4 py-2 text-sm font-semibold text-white"
                    onClick={handleCreateAccount}
                  >
                    Create
                  </button>
                </div>
              </div>

              <div>
                <label className="text-xs font-semibold uppercase text-zinc-500">
                  Import account
                </label>
                <div className="mt-2 grid gap-2">
                  <input
                    className="rounded-md border border-zinc-200 px-3 py-2 text-sm"
                    value={importName}
                    onChange={(event) => setImportName(event.target.value)}
                    placeholder="name"
                  />
                  <input
                    className="rounded-md border border-zinc-200 px-3 py-2 text-sm"
                    value={importSecret}
                    onChange={(event) => setImportSecret(event.target.value)}
                    placeholder="secret hex"
                  />
                  <button
                    className="rounded-md border border-zinc-200 px-4 py-2 text-sm"
                    onClick={handleImportAccount}
                  >
                    Import
                  </button>
                </div>
              </div>
            </div>
          </div>

          <div className="flex flex-col gap-6">
            <div className="rounded-lg border border-zinc-200 bg-white p-5">
              <h2 className="text-lg font-semibold">Selected Account</h2>
              <p className="mt-2 text-sm text-zinc-500">
                {selectedAccount
                  ? `${selectedAccount} • ${selectedAddress}`
                  : "Select an account to continue."}
              </p>
              <p className="mt-2 text-sm text-zinc-700">
                Balance: {selectedBalance} MIRA
              </p>
              <div className="mt-4 flex flex-wrap gap-2">
                <button
                  className="rounded-md border border-zinc-200 px-4 py-2 text-sm"
                  onClick={handleCopyAddress}
                >
                  Copy Address
                </button>
                <button
                  className="rounded-md border border-zinc-200 px-4 py-2 text-sm"
                  onClick={handleExportSecret}
                >
                  Export Secret
                </button>
              </div>
              {exportSecret && (
                <div className="mt-3 rounded-md border border-amber-200 bg-amber-50 px-3 py-2 text-xs text-amber-800">
                  Secret key: {exportSecret}
                </div>
              )}
            </div>

            <div className="rounded-lg border border-zinc-200 bg-white p-5">
              <h2 className="text-lg font-semibold">Send MIRA</h2>
              <div className="mt-3 grid gap-2">
                <input
                  className="rounded-md border border-zinc-200 px-3 py-2 text-sm"
                  value={transferTo}
                  onChange={(event) => setTransferTo(event.target.value)}
                  placeholder="Recipient address"
                />
                <input
                  className="rounded-md border border-zinc-200 px-3 py-2 text-sm"
                  value={transferAmount}
                  onChange={(event) => setTransferAmount(event.target.value)}
                  placeholder="Amount"
                  type="number"
                  min="0"
                />
                <button
                  className="rounded-md bg-zinc-900 px-4 py-2 text-sm font-semibold text-white"
                  onClick={handleTransfer}
                >
                  Send
                </button>
              </div>
            </div>

            <div className="rounded-lg border border-zinc-200 bg-white p-5">
              <h2 className="text-lg font-semibold">Receive MIRA</h2>
              <p className="mt-2 text-sm text-zinc-600">
                Share this address with the sender:
              </p>
              <div className="mt-2 rounded-md border border-dashed border-zinc-300 bg-zinc-50 px-3 py-2 text-xs">
                {selectedAddress || "Select an account."}
              </div>
            </div>
          </div>
        </section>

        {status.message && (
          <section
            className={`rounded-md px-4 py-2 text-sm ${
              status.kind === "error"
                ? "bg-red-50 text-red-700"
                : status.kind === "success"
                ? "bg-emerald-50 text-emerald-700"
                : "bg-zinc-100 text-zinc-700"
            }`}
          >
            {status.message}
          </section>
        )}
      </div>
    </div>
  );
}
