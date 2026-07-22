# roe

Codebase intelligence for C#. Finds dead code — unused types, members, and
files — and duplicated code across your solution. Static analysis only; roe
never runs the code it analyzes.

```
npm install --global roe-cli
roe dead-code path/to/solution
roe dupes path/to/solution
```

Or run it one-shot without installing:

```
npx roe-cli dead-code .
```

This package bundles prebuilt binaries for Linux (x64/arm64), macOS
(x64/arm64), and Windows (x64) and runs the one matching your platform.

Full documentation: https://github.com/Artmann/roe
