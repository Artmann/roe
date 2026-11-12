namespace App;

internal sealed class Node
{
    public bool Lit { get; set; }
}

// C# 14 null-conditional assignment. The grammar predates it, and its error
// recovery turns the following lines into fields named `` and `true`.
internal sealed class Highlighter
{
    private Node? _lit;

    public bool Light(Node? next)
    {
        _lit?.Lit = false;
        next?.Lit = true;
        _lit = next;
        return true;
    }
}

internal static class Program
{
    private static void Main()
    {
        new Highlighter().Light(new Node());
    }
}
