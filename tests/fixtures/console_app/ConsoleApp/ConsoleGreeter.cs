namespace ConsoleApp;

public class ConsoleGreeter : IGreeter
{
    public string Greet(string name)
    {
        return Format(name);
    }

    private string Format(string name)
    {
        return $"Hello, {name}!";
    }

    private string UnusedHelper(string name)
    {
        return name.ToUpperInvariant();
    }
}
