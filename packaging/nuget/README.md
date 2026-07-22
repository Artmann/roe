# roe

Codebase intelligence for C#. Finds dead code — unused types, members, and
files — and duplicated code across your solution. Static analysis only; roe
never runs the code it analyzes.

```
dotnet tool install --global roe
roe dead-code path/to/solution
roe dupes path/to/solution
```

Or run it one-shot without installing (.NET 10 SDK or later):

```
dnx roe dead-code .
```

The tool bundles prebuilt binaries for linux-x64, linux-arm64, osx-x64,
osx-arm64, and win-x64.

Full documentation: https://github.com/Artmann/roe
