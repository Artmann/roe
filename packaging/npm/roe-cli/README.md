# roe

Codebase intelligence for C#. Finds dead code — unused types, members, and
files — duplicated code, and complexity/coupling hotspots, so you can delete
what's unused, de-duplicate what's copy-pasted, and clean up what's hard to
work with. Static analysis only; roe never runs the code it analyzes.

```
npm install --global roe-cli
roe path/to/solution
```

With no command, roe runs all three analyses and prints each report under its
own section header. Run one at a time when you want to focus, or tune its
thresholds:

```
roe dead-code path/to/solution
roe dupes path/to/solution
roe health path/to/solution
```

Or run it one-shot without installing:

```
npx roe-cli .
```

This package bundles prebuilt binaries for Linux (x64/arm64), macOS
(x64/arm64), and Windows (x64) and runs the one matching your platform.

Full documentation: https://github.com/Artmann/roe
