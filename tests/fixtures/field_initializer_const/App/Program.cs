namespace App;

// Each constant is used only in a field initializer. A and B sit in a call
// argument next to a primary-constructor parameter, C and D are controls.
internal sealed class CastCallParam(int ticks)
{
    private const float A = 0.5f;
    private readonly int _a = (int)MathF.Ceiling(A * ticks);
    public int Value => _a;
}

internal sealed class CallParam(int ticks)
{
    private const float B = 0.5f;
    private readonly float _b = MathF.Ceiling(B * ticks);
    public float Value => _b;
}

internal sealed class CastParam(int ticks)
{
    private const float C = 0.5f;
    private readonly int _c = (int)(C * ticks);
    public int Value => _c;
}

internal sealed class CastCallNoParam
{
    private const float D = 0.5f;
    private readonly int _d = (int)MathF.Ceiling(D * 60);
    public int Value => _d;
}

internal static class Program
{
    private static void Main()
    {
        Console.WriteLine(new CastCallParam(3).Value + new CallParam(3).Value + new CastParam(3).Value + new CastCallNoParam().Value);
    }
}
