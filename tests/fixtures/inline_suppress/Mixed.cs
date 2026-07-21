namespace InlineSuppress;

public class LiveMarker
{
    public void Ping()
    {
    }
}

// roe-ignore-next-line unused-type
internal class SuppressedType
{
    public void DoNothing()
    {
    }
}

internal class TrulyDeadType
{
    public void DoNothing()
    {
    }
}

// roe-ignore-next-line unused-member
internal class WrongRuleType
{
    public void DoNothing()
    {
    }
}
