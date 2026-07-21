namespace InlineSuppress;

public static class Program
{
    public static void Main()
    {
        var marker = new LiveMarker();
        marker.Ping();

        var helper = new HelperHost();
        helper.UsedMethod();
    }
}
