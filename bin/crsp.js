#!/usr/bin/env node

const path = require('path');
const fs = require('fs');
const { spawnSync } = require('child_process');

const exeName = process.platform === 'win32' ? 'crsp-bin.exe' : 'crsp-bin';
const binPath = path.join(__dirname, exeName);

if (!fs.existsSync(binPath)) {
  console.error(`Error: crsp binary was not found at: ${binPath}`);
  console.error(`Please run "npm rebuild @sinsky-gh/crsp" or reinstall with "npm install -g @sinsky-gh/crsp"`);
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
