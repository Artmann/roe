namespace Lib;

public enum StepKind
{
    Opaque,
    Glass,
}

public static class Tiles
{
    public static int Bricks(int columns) => columns * 2;

    public static int Count(StepKind kind) => kind == StepKind.Glass ? Bricks(2) : 1;
}
