namespace App;

// A static factory and the generic type it creates share a name. The cycle
// between them must not read as `App.Tween → App.Tween`.
internal static class Tween
{
    public static Tween<float> Between(float from, float to) => new(from, to);

    public static Tween<T> Reverse<T>(Tween<T> tween) => new(tween.To, tween.From);
}

internal readonly struct Tween<T>(T from, T to)
{
    public T From => from;

    public T To => to;

    public Tween<T> Reversed() => Tween.Reverse(this);
}

// The same shape with two type parameters.
internal static class Pair
{
    public static Pair<TFirst, TSecond> Of<TFirst, TSecond>(TFirst first, TSecond second) => new(first, second);
}

internal readonly struct Pair<TFirst, TSecond>(TFirst first, TSecond second)
{
    public TFirst First => first;

    public TSecond Second => second;

    public Pair<TSecond, TFirst> Swapped() => Pair.Of(second, first);
}

// A type nested in a generic type carries the outer type's parameters.
internal sealed class Pool<T>
{
    public sealed class Slot(Registry registry)
    {
        public Registry Registry => registry;
    }
}

internal sealed class Registry
{
    public Pool<int>.Slot? Last { get; set; }
}

internal static class Program
{
    private static void Main()
    {
        Console.WriteLine(Tween.Between(0f, 1f).Reversed().From);
        Console.WriteLine(Pair.Of(1, "one").Swapped().First);
    }
}
