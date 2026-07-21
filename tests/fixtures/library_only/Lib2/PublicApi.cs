namespace Lib2;

public class PublicApi
{
    private int neverUsed;

    public int Answer()
    {
        return Secret();
    }

    private int Secret()
    {
        return 42;
    }
}
