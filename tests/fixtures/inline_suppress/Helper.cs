namespace InlineSuppress;

public class HelperHost
{
    public void UsedMethod()
    {
    }

    // roe-ignore-next-line
    private void BareSuppressed()
    {
    }

    private void TrailingSuppressed() // roe-ignore-line unused-member
    {
    }

    private void UnannotatedDead()
    {
    }
}
