const fs = require('fs');
const path = require('path');
const os = require('os');
const { execSync } = require('child_process');

const pkg = require('../package.json');
const version = pkg.version;

const PLATFORM_MAP = {
  'darwin-arm64': {
    target: 'aarch64-apple-darwin',
    archive: `crsp-aarch64-apple-darwin.tar.gz`,
    binary: 'crsp-aarch64-apple-darwin/crsp',
  },
  'linux-x64': {
    target: 'x86_64-unknown-linux-musl',
    archive: `crsp-x86_64-unknown-linux-musl.tar.gz`,
    binary: 'crsp-x86_64-unknown-linux-musl/crsp',
  },
  'linux-arm64': {
    target: 'aarch64-unknown-linux-musl',
    archive: `crsp-aarch64-unknown-linux-musl.tar.gz`,
    binary: 'crsp-aarch64-unknown-linux-musl/crsp',
  },
  'win32-x64': {
    target: 'x86_64-pc-windows-msvc',
    archive: `crsp-x86_64-pc-windows-msvc.zip`,
    binary: 'crsp-x86_64-pc-windows-msvc/crsp.exe',
  },
};

const key = `${process.platform}-${process.arch}`;
const targetInfo = PLATFORM_MAP[key];

if (!targetInfo) {
  console.warn(`[crsp] Warning: Unsupported platform or architecture: ${key}`);
  console.warn(`[crsp] Supported targets: ${Object.keys(PLATFORM_MAP).join(', ')}`);
  console.warn(`[crsp] You can compile from source with "cargo install google-clasp-rs" or download a release from GitHub (sinsky/crsp).`);
  process.exit(0);
}

const downloadUrl = `https://github.com/sinsky/crsp/releases/download/v${version}/${targetInfo.archive}`;
const binDir = path.join(__dirname, '..', 'bin');
const destBinaryName = process.platform === 'win32' ? 'crsp-bin.exe' : 'crsp-bin';
const destBinaryPath = path.join(binDir, destBinaryName);

async function main() {
  if (fs.existsSync(destBinaryPath)) {
    // Already downloaded
    return;
  }

  console.log(`[crsp] Downloading prebuilt binary for ${key} from ${downloadUrl}...`);

  const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'crsp-install-'));
  const archivePath = path.join(tmpDir, targetInfo.archive);

  try {
    const res = await fetch(downloadUrl, { redirect: 'follow' });
    if (!res.ok) {
      throw new Error(`Failed to download binary: HTTP ${res.status} ${res.statusText}`);
    }

    const arrayBuffer = await res.arrayBuffer();
    fs.writeFileSync(archivePath, Buffer.from(arrayBuffer));

    if (targetInfo.archive.endsWith('.zip')) {
      try {
        execSync(`tar -xf "${archivePath}" -C "${tmpDir}"`, { stdio: 'pipe' });
      } catch {
        execSync(`powershell -Command "Expand-Archive -Path '${archivePath}' -DestinationPath '${tmpDir}'"`, { stdio: 'inherit' });
      }
    } else {
      execSync(`tar -xzf "${archivePath}" -C "${tmpDir}"`, { stdio: 'pipe' });
    }

    const extractedBinary = path.join(tmpDir, targetInfo.binary);
    if (!fs.existsSync(extractedBinary)) {
      throw new Error(`Extracted binary not found at expected path: ${extractedBinary}`);
    }

    if (!fs.existsSync(binDir)) {
      fs.mkdirSync(binDir, { recursive: true });
    }

    fs.copyFileSync(extractedBinary, destBinaryPath);
    if (process.platform !== 'win32') {
      fs.chmodSync(destBinaryPath, 0o755);
    }

    console.log(`[crsp] Successfully installed crsp v${version}`);
  } finally {
    try {
      fs.rmSync(tmpDir, { recursive: true, force: true });
    } catch {
      // ignore cleanup errors
    }
  }
}

main().catch((err) => {
  console.error(`[crsp] Installation failed: ${err.message}`);
  // Do not fail hard during postinstall if offline, but notify
  process.exit(1);
});
