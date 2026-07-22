#!/usr/bin/env node
'use strict';

const { spawnSync } = require('child_process');
const fs = require('fs');
const path = require('path');

const supportedPlatforms = ['darwin-arm64', 'darwin-x64', 'linux-arm64', 'linux-x64', 'win32-x64'];

const platformKey = `${process.platform}-${process.arch}`;

if (!supportedPlatforms.includes(platformKey)) {
  console.error(
    `roe doesn't ship a prebuilt binary for ${platformKey}. ` +
      `Supported platforms: ${supportedPlatforms.join(', ')}. ` +
      'Build from source with `cargo install --git https://github.com/Artmann/roe` ' +
      'or open an issue at https://github.com/Artmann/roe/issues.'
  );
  process.exit(2);
}

const binaryName = process.platform === 'win32' ? 'roe.exe' : 'roe';
const binaryPath = path.join(__dirname, '..', 'binaries', platformKey, binaryName);

if (!fs.existsSync(binaryPath)) {
  console.error(
    `roe's bundled binary is missing at ${binaryPath}. The package appears to be corrupted. ` +
      'Reinstall with `npm install roe-cli` ' +
      'or open an issue at https://github.com/Artmann/roe/issues.'
  );
  process.exit(2);
}

const result = spawnSync(binaryPath, process.argv.slice(2), { stdio: 'inherit' });

if (result.error) {
  console.error(
    `roe failed to start (${result.error.message}). Reinstall with \`npm install roe-cli\` ` +
      'or open an issue at https://github.com/Artmann/roe/issues.'
  );
  process.exit(2);
}

process.exit(result.status ?? 1);
