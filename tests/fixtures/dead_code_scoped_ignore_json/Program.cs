namespace Messaging;

public static class Program
{
    public static void Main()
    {
        var pings = new Services.PingService();
        pings.PingAsync("gateway-1").Wait();
    }
}
