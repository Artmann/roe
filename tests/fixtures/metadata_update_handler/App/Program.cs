using System.Reflection.Metadata;

[assembly: MetadataUpdateHandler(typeof(App.HotReload))]

namespace App;

// The runtime calls ClearCache and UpdateApplication by name after every hot
// reload update. Nothing in source calls them.
internal static class HotReload
{
    private static int _changes;

    public static int Changes => Volatile.Read(ref _changes);

    internal static void ClearCache(Type[]? updatedTypes)
    {
    }

    internal static void UpdateApplication(Type[]? updatedTypes) => Interlocked.Increment(ref _changes);

    // Not part of the hot reload contract, so still dead.
    internal static void Unused()
    {
    }
}

// Same method names on a type that is not a handler: still dead.
internal static class NotAHandler
{
    public static int Value => 1;

    internal static void ClearCache(Type[]? updatedTypes)
    {
    }
}

internal static class Program
{
    private static void Main()
    {
        Console.WriteLine(HotReload.Changes + NotAHandler.Value);
    }
}
