using System.Numerics;
using System.Runtime.CompilerServices;

namespace App;

// The single instance field is the array's element storage. Every indexer and
// span access reads and writes it, but nothing ever names it.
[InlineArray(Length)]
internal struct Parameters
{
    public const int Length = 8;
    private Vector4 _element;
}

// The same, with the fully qualified attribute name.
[System.Runtime.CompilerServices.InlineArrayAttribute(4)]
internal struct Slots
{
    private int _slot;
}

internal static class Program
{
    private static void Main()
    {
        var parameters = new Parameters();
        parameters[0] = Vector4.One;
        Console.WriteLine(parameters[0]);

        var slots = new Slots();
        slots[1] = 2;
        Console.WriteLine(slots[1]);
    }
}
