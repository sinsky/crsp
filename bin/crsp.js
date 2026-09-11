#!/usr/bin/env node

const path = require('path');
const fs = require('fs');
const { spawnSync } = require('child_process');

const PLATFORM_PACKAGES = [
  'darwin-arm64',
  'darwin-x64',
  'linux-x64',
  'linux-arm64',
  'win32-x64',
];

const exeName = process.platform === 'win32' ? 'crsp-bin.exe' : 'crsp-bin';

function resolveBinary() {
  for (const platform of PLATFORM_PACKAGES) {
    const pkgName = `@sinsky-gh/crsp-${platform}`;
    try {
      const pkgPath = path.dirname(require.resolve(`${pkgName}/package.json`));
      const binPath = path.join(pkgPath, 'bin', exeName);
      if (fs.existsSync(binPath)) {
        return binPath;
      }
    } catch {
      // not installed for this platform, try next
    }
  }
  return null;
}

const binPath = resolveBinary();

if (!binPath) {
  console.error(`Error: crsp binary was not found for ${process.platform}-${process.arch}`);
  console.error(`Installed platform packages: ${PLATFORM_PACKAGES.join(', ')} (only matching os/cpu is installed)`);
  console.error(`Please reinstall with "npm install -g @sinsky-gh/crsp"`);
  process.exit(1);
}

const result = spawnSync(binPath, process.argv.slice(2), {
  stdio: 'inherit'
});

if (result.error) {
  console.error(result.error.message);
  process.exit(1);
}

process.exit(result.status ?? 0);
