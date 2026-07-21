namespace Kill;

public static class StringExtensions
{
    public static string Shout(this string value)
    {
        return value.ToUpperInvariant();
    }
}
