namespace ConsoleApp;

public static class Program
{
    public static void Main(string[] args)
    {
        IGreeter greeter = new ConsoleGreeter();
        Console.WriteLine(greeter.Greet("world"));
    }
}
