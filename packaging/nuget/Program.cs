using System.Diagnostics;
using System.Runtime.InteropServices;

// This tool is a thin launcher: the actual roe CLI is a Rust binary bundled
// per platform under binaries/<rid>/ in the NuGet package. The shim exists so
// `dotnet tool install -g roe` works on any .NET SDK from 8.0 up.

var runtimeFolder = ResolveRuntimeFolder();

if (runtimeFolder is null)
{
    Console.Error.WriteLine(
        "roe doesn't ship a prebuilt binary for this platform " +
        $"({RuntimeInformation.OSDescription}, {RuntimeInformation.ProcessArchitecture}). " +
        "Supported platforms: linux-x64, linux-arm64, osx-x64, osx-arm64, win-x64. " +
        "Build from source with `cargo install --git https://github.com/Artmann/roe` " +
        "or open an issue at https://github.com/Artmann/roe/issues.");

    return 2;
}

var binaryName = OperatingSystem.IsWindows() ? "roe.exe" : "roe";
var binaryPath = Path.Combine(AppContext.BaseDirectory, "binaries", runtimeFolder, binaryName);

if (!File.Exists(binaryPath))
{
    Console.Error.WriteLine(
        $"roe's bundled binary is missing at {binaryPath}. The tool package appears to be " +
        "corrupted. Reinstall with `dotnet tool update -g roe`.");

    return 2;
}

if (!OperatingSystem.IsWindows())
{
    // NuGet package extraction doesn't preserve Unix execute bits
    // (NuGet/Home#13402), so set them on first run.
    try
    {
        var mode = File.GetUnixFileMode(binaryPath);
        var executable = UnixFileMode.UserExecute | UnixFileMode.GroupExecute | UnixFileMode.OtherExecute;

        if ((mode & executable) != executable)
        {
            File.SetUnixFileMode(binaryPath, mode | executable);
        }
    }
    catch (Exception exception)
    {
        Console.Error.WriteLine(
            $"roe couldn't mark its bundled binary as executable ({exception.Message}). " +
            $"Fix it manually with `chmod +x \"{binaryPath}\"` and run roe again.");

        return 2;
    }
}

var startInfo = new ProcessStartInfo(binaryPath)
{
    UseShellExecute = false
};

foreach (var argument in args)
{
    startInfo.ArgumentList.Add(argument);
}

// Let Ctrl+C flow to the child (which shares the console) instead of killing
// the shim first — the child's exit code is the contract (0 clean, 1
// findings, 2 error).
Console.CancelKeyPress += (_, eventArguments) => eventArguments.Cancel = true;

using var process = Process.Start(startInfo);

if (process is null)
{
    Console.Error.WriteLine(
        $"roe failed to start its bundled binary at {binaryPath}. " +
        "Reinstall with `dotnet tool update -g roe` " +
        "or open an issue at https://github.com/Artmann/roe/issues.");

    return 2;
}

process.WaitForExit();

return process.ExitCode;

static string? ResolveRuntimeFolder()
{
    var architecture = RuntimeInformation.ProcessArchitecture switch
    {
        Architecture.Arm64 => "arm64",
        Architecture.X64 => "x64",
        _ => null
    };

    if (architecture is null)
    {
        return null;
    }

    if (OperatingSystem.IsLinux())
    {
        return $"linux-{architecture}";
    }

    if (OperatingSystem.IsMacOS())
    {
        return $"osx-{architecture}";
    }

    if (OperatingSystem.IsWindows() && architecture == "x64")
    {
        return "win-x64";
    }

    return null;
}
