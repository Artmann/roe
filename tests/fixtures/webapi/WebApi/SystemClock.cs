namespace WebApi;

public class SystemClock : IClock
{
    public DateTime Now()
    {
        return DateTime.UtcNow;
    }
}
