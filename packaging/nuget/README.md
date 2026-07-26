# roe

Codebase intelligence for C#. Finds dead code — unused types, members, and
files — duplicated code, and complexity/coupling hotspots, so you can delete
what's unused, de-duplicate what's copy-pasted, and clean up what's hard to
work with. Static analysis only; roe never runs the code it analyzes.

```
dotnet tool install --global roe
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

Or run it one-shot without installing (.NET 10 SDK or later):

```
dnx roe .
```

The tool bundles prebuilt binaries for linux-x64, linux-arm64, osx-x64,
osx-arm64, and win-x64.

Full documentation: https://github.com/Artmann/roe
