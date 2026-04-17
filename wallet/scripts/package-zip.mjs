#!/usr/bin/env node
import { execFile } from "node:child_process";
import { existsSync } from "node:fs";
import { mkdir, rm, copyFile, writeFile, readFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

const args = new Set(process.argv.slice(2));
const isDryRun = args.has("--dry-run");

const walletRoot = path.resolve(__dirname, "..");
const repoRoot = path.resolve(walletRoot, "..");
const distDir = path.join(walletRoot, "dist");
const stagingDir = path.join(distDir, "wallet-miraset");
const tauriConfPath = path.join(walletRoot, "src-tauri", "tauri.conf.json");
const defaultExePath = path.join(
  walletRoot,
  "src-tauri",
  "target",
  "release",
  "wallet-miraset.exe"
);

const readmeContent = `MIRASET Desktop Wallet\n\nRun: wallet-miraset.exe\nRPC default: http://127.0.0.1:9944\nWallet file: %USERPROFILE%\\.miraset\\wallet.json\n\nIf the app fails to start, install WebView2 runtime (Windows).\n`;

function log(line) {
  process.stdout.write(`${line}\n`);
}

function execPowerShell(command, argsList) {
  return new Promise((resolve, reject) => {
    execFile(command, argsList, { windowsHide: true }, (error, stdout, stderr) => {
      if (error) {
        reject(new Error(stderr || error.message));
        return;
      }
      resolve(stdout.trim());
    });
  });
}

async function readTauriVersion() {
  try {
    const raw = await readFile(tauriConfPath, "utf8");
    const parsed = JSON.parse(raw);
    return parsed?.package?.version || "0.0.0";
  } catch {
    return "0.0.0";
  }
}

async function main() {
  const version = await readTauriVersion();
  const zipName = `wallet-miraset-${version}-win64.zip`;
  const zipPath = path.join(distDir, zipName);

  log(`Staging directory: ${stagingDir}`);
  log(`Output ZIP: ${zipPath}`);

  if (isDryRun) {
    log("Dry run enabled. No files will be created.");
    return;
  }

  if (!existsSync(defaultExePath)) {
    throw new Error(
      `Executable not found at ${defaultExePath}. Run 'bun run tauri:build' first.`
    );
  }

  await rm(stagingDir, { recursive: true, force: true });
  await mkdir(stagingDir, { recursive: true });

  await copyFile(defaultExePath, path.join(stagingDir, "wallet-miraset.exe"));
  await writeFile(path.join(stagingDir, "README.txt"), readmeContent, "utf8");

  const licensePath = path.join(repoRoot, "LICENSE");
  if (existsSync(licensePath)) {
    await copyFile(licensePath, path.join(stagingDir, "LICENSE"));
  }

  await mkdir(distDir, { recursive: true });

  await execPowerShell("powershell.exe", [
    "-NoProfile",
    "-Command",
    `Compress-Archive -Path "${stagingDir}\\*" -DestinationPath "${zipPath}" -Force`,
  ]);

  log("ZIP package created successfully.");
}

main().catch((error) => {
  process.stderr.write(`Packaging failed: ${error.message}\n`);
  process.exit(1);
});

