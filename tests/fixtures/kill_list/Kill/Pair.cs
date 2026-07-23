namespace Kill;

public class Pair
{
    private readonly int first;
    private readonly int second;

    public Pair(int first, int second)
    {
        this.first = first;
        this.second = second;
    }

    public void Deconstruct(out int first, out int second)
    {
        first = this.first;
        second = this.second;
    }
}
