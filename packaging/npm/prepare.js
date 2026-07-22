#!/usr/bin/env node
'use strict';

// Assembles the publishable roe-cli npm package under packaging/npm/build/:
// the launcher plus the prebuilt binary for every supported platform, with
// the version stamped from the release tag. Run by the release workflow as:
//
//   node packaging/npm/prepare.js <version> <binaries-directory>
//
// where <binaries-directory> holds one bin-<target> folder per platform, as
// produced by actions/download-artifact.

const fs = require('fs');
const path = require('path');

const platforms = [
  { binary: 'roe', name: 'darwin-arm64', target: 'aarch64-apple-darwin' },
  { binary: 'roe', name: 'darwin-x64', target: 'x86_64-apple-darwin' },
  { binary: 'roe', name: 'linux-arm64', target: 'aarch64-unknown-linux-gnu' },
  { binary: 'roe', name: 'linux-x64', target: 'x86_64-unknown-linux-gnu' },
  { binary: 'roe.exe', name: 'win32-x64', target: 'x86_64-pc-windows-msvc' }
];

const repositoryRoot = path.join(__dirname, '..', '..');
const buildDirectory = path.join(__dirname, 'build');
const packageDirectory = path.join(buildDirectory, 'roe-cli');

function assertSemver(version) {
  if (!version || !/^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$/.test(version)) {
    console.error('Usage: node packaging/npm/prepare.js <version> <binaries-directory>');
    console.error(`The version must be a semver string like 0.2.0, got "${version ?? ''}".`);
    process.exit(2);
  }
}

function copyBinary(platform, binariesDirectory) {
  const sourceBinary = path.join(binariesDirectory, `bin-${platform.target}`, platform.binary);

  if (!fs.existsSync(sourceBinary)) {
    console.error(`Missing binary for ${platform.name}: expected ${sourceBinary}.`);
    console.error('Did the build job for this target succeed and upload its artifact?');
    process.exit(2);
  }

  const destination = path.join(packageDirectory, 'binaries', platform.name, platform.binary);

  fs.mkdirSync(path.dirname(destination), { recursive: true });
  fs.copyFileSync(sourceBinary, destination);
  fs.chmodSync(destination, 0o755);
}

function writeManifest(version) {
  const sourceDirectory = path.join(__dirname, 'roe-cli');
  const manifest = JSON.parse(fs.readFileSync(path.join(sourceDirectory, 'package.json'), 'utf8'));

  manifest.version = version;

  fs.mkdirSync(path.join(packageDirectory, 'bin'), { recursive: true });
  fs.copyFileSync(path.join(sourceDirectory, 'bin', 'roe.js'), path.join(packageDirectory, 'bin', 'roe.js'));
  fs.copyFileSync(path.join(sourceDirectory, 'README.md'), path.join(packageDirectory, 'README.md'));
  fs.copyFileSync(path.join(repositoryRoot, 'LICENSE'), path.join(packageDirectory, 'LICENSE'));
  fs.writeFileSync(path.join(packageDirectory, 'package.json'), `${JSON.stringify(manifest, null, 2)}\n`);
}

function main() {
  const version = process.argv[2];
  const binariesDirectory = process.argv[3] ?? 'binaries';

  assertSemver(version);

  if (!fs.existsSync(binariesDirectory)) {
    console.error(`The binaries directory "${binariesDirectory}" doesn't exist.`);
    console.error('Download the bin-* artifacts first (see .github/workflows/release.yml).');
    process.exit(2);
  }

  fs.rmSync(buildDirectory, { force: true, recursive: true });

  for (const platform of platforms) {
    copyBinary(platform, binariesDirectory);
  }

  writeManifest(version);

  console.log(`Assembled roe-cli@${version} with ${platforms.length} platform binaries in ${packageDirectory}.`);
}

main();
