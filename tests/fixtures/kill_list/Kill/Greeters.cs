namespace Kill;

public interface IGreeter
{
    void Greet();
}

public class PoliteGreeter : IGreeter
{
    public void Greet()
    {
        Console.WriteLine("Hello");
    }
}
