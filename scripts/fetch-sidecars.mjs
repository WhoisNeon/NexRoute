#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import https from "node:https";

const ROOT = path.resolve(process.cwd(), "src-tauri", "binaries");
const TARGET = process.env.TARGET || targetTriple();

function targetTriple() {
  const platform = process.platform;
  const arch = process.arch;
  if (platform === "win32") {
    if (arch === "x64") return "x86_64-pc-windows-msvc";
    if (arch === "arm64") return "aarch64-pc-windows-msvc";
  }
  if (platform === "darwin") {
    if (arch === "x64") return "x86_64-apple-darwin";
    if (arch === "arm64") return "aarch64-apple-darwin";
  }
  if (platform === "linux") {
    if (arch === "x64") return "x86_64-unknown-linux-gnu";
    if (arch === "arm64") return "aarch64-unknown-linux-gnu";
  }
  throw new Error(`unsupported platform/arch: ${platform}/${arch}`);
}

function isWindowsTarget(target) {
  return target.includes("windows");
}
function ext(target) {
  return isWindowsTarget(target) ? ".exe" : "";
}

function request(url, headers = {}) {
  return new Promise((resolve, reject) => {
    const req = https.get(url, { headers: { "User-Agent": "NexRoute-sidecar-fetch", ...headers } }, (res) => {
      if (res.statusCode && res.statusCode >= 300 && res.statusCode < 400 && res.headers.location) {
        resolve(request(res.headers.location, headers));
        return;
      }
      if (!res.statusCode || res.statusCode < 200 || res.statusCode >= 300) {
        reject(new Error(`HTTP ${res.statusCode} for ${url}`));
        return;
      }
      const chunks = [];
      res.on("data", (chunk) => chunks.push(chunk));
      res.on("end", () => resolve(Buffer.concat(chunks)));
    });
    req.on("error", reject);
  });
}

async function githubRelease(repo) {
  const token = process.env.GITHUB_TOKEN || process.env.GH_TOKEN;
  const headers = token ? { Authorization: `Bearer ${token}` } : {};
  const body = await request(`https://api.github.com/repos/${repo}/releases/latest`, {
    ...headers,
    Accept: "application/vnd.github+json",
  });
  return JSON.parse(body.toString("utf8"));
}

async function download(url, dest) {
  const data = await request(url);
  fs.mkdirSync(path.dirname(dest), { recursive: true });
  fs.writeFileSync(dest, data);
  return dest;
}

function sha256File(file) {
  return crypto.createHash("sha256").update(fs.readFileSync(file)).digest("hex");
}

function findAsset(assets, predicate) {
  const asset = assets.find((a) => predicate(a.name));
  if (!asset) throw new Error(`missing release asset for target ${TARGET}`);
  return asset;
}

function tempDir() {
  return fs.mkdtempSync(path.join(os.tmpdir(), "nexroute-sidecar-"));
}

function extractArchive(archive, dest, archiveType) {
  fs.mkdirSync(dest, { recursive: true });
  if (archiveType === "tar.gz") {
    const result = spawnSync("tar", ["-xzf", archive, "-C", dest], { stdio: "inherit" });
    if (result.status !== 0) throw new Error("tar extraction failed");
    return;
  }
  if (archiveType === "zip") {
    if (process.platform === "win32") {
      const ps = `Expand-Archive -LiteralPath '${archive}' -DestinationPath '${dest}' -Force`;
      const result = spawnSync("powershell", ["-NoProfile", "-Command", ps], { stdio: "inherit" });
      if (result.status !== 0) throw new Error("zip extraction failed");
    } else {
      const result = spawnSync("unzip", ["-o", archive, "-d", dest], { stdio: "inherit" });
      if (result.status !== 0) throw new Error("unzip extraction failed");
    }
    return;
  }
  throw new Error(`unknown archive type ${archiveType}`);
}

function findBinary(root, names) {
  const stack = [root];
  while (stack.length) {
    const current = stack.pop();
    for (const entry of fs.readdirSync(current, { withFileTypes: true })) {
      const full = path.join(current, entry.name);
      if (entry.isDirectory()) stack.push(full);
      else if (names.includes(entry.name.toLowerCase())) return full;
    }
  }
  throw new Error(`binary not found after extraction (${names.join(", ")})`);
}

async function fetchSingBox() {
  const release = await githubRelease("SagerNet/sing-box");
  const os = TARGET.includes("windows")
    ? "windows"
    : TARGET.includes("darwin")
      ? "darwin"
      : "linux";
  const arch = TARGET.startsWith("aarch64") ? "arm64" : "amd64";
  const needle = `${os}-${arch}`;
  const archiveAsset = findAsset(release.assets, (name) =>
    name.includes("sing-box") && name.includes(needle) && (name.endsWith(".tar.gz") || name.endsWith(".zip")),
  );
  const checksumAsset = release.assets.find((a) => a.name === `${archiveAsset.name}.sha256sum`);

  const tmp = tempDir();
  const archivePath = path.join(tmp, archiveAsset.name);
  await download(archiveAsset.browser_download_url, archivePath);

  if (checksumAsset) {
    const checksumText = (await request(checksumAsset.browser_download_url)).toString("utf8");
    const expected = checksumText.trim().split(/\s+/)[0];
    const actual = sha256File(archivePath);
    if (expected && expected.length === 64 && expected !== actual) {
      throw new Error(`sing-box checksum mismatch: ${actual} != ${expected}`);
    }
  }

  const extractDir = path.join(tmp, "extract");
  extractArchive(archivePath, extractDir, archiveAsset.name.endsWith(".zip") ? "zip" : "tar.gz");
  const source = findBinary(extractDir, [isWindowsTarget(TARGET) ? "sing-box.exe" : "sing-box"]);
  const destination = path.join(ROOT, `sing-box-${TARGET}${ext(TARGET)}`);
  fs.mkdirSync(ROOT, { recursive: true });
  fs.copyFileSync(source, destination);
  if (!isWindowsTarget(TARGET)) fs.chmodSync(destination, 0o755);
  console.log(`fetched sing-box -> ${destination}`);
}

async function fetchXray() {
  const release = await githubRelease("XTLS/Xray-core");
  let assetName;
  if (TARGET.includes("windows")) assetName = "Xray-windows-64.zip";
  else if (TARGET.includes("darwin")) assetName = TARGET.startsWith("aarch64") ? "Xray-macos-arm64-v8a.zip" : "Xray-macos-64.zip";
  else assetName = TARGET.startsWith("aarch64") ? "Xray-linux-arm64-v8a.zip" : "Xray-linux-64.zip";

  const asset = findAsset(release.assets, (name) => name === assetName);

  const tmp = tempDir();
  const archivePath = path.join(tmp, asset.name);
  await download(asset.browser_download_url, archivePath);

  const extractDir = path.join(tmp, "extract");
  extractArchive(archivePath, extractDir, "zip");
  const source = findBinary(extractDir, [isWindowsTarget(TARGET) ? "xray.exe" : "xray"]);
  const destination = path.join(ROOT, `xray-${TARGET}${ext(TARGET)}`);
  fs.mkdirSync(ROOT, { recursive: true });
  fs.copyFileSync(source, destination);
  if (!isWindowsTarget(TARGET)) fs.chmodSync(destination, 0o755);
  console.log(`fetched xray -> ${destination}`);
}

fs.mkdirSync(ROOT, { recursive: true });
await fetchSingBox();
await fetchXray();
console.log(`done for ${TARGET}`);