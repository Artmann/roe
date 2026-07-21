namespace ConfigExplicit;

public enum Status
{
    Active,
    Legacy,
}

public static class Program
{
    public static void Main()
    {
        var s = Status.Active;
    }
}
