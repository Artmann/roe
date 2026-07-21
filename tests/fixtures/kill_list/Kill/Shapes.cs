namespace Kill;

public abstract class Shape
{
    public abstract void Render();
}

public class Circle : Shape
{
    public override void Render()
    {
        Console.WriteLine("o");
    }
}
