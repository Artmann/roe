using Lib;

// Both classes live in the global namespace, in a project that Lib does not
// reference. Lib's `Tiles.Bricks(...)` method and `StepKind.Glass` enum member
// share their names, but can't refer to them.
public sealed class Bricks
{
    public int Make() => Tiles.Bricks(4);
}

public sealed class Glass
{
    public int Make() => Tiles.Count(StepKind.Opaque);
}

internal static class Program
{
    private static void Main()
    {
        Console.WriteLine(Tiles.Count(StepKind.Glass));
    }
}
